//! [`load`]: Lottie JSON → [`Composition`].
//!
//! The JSON is parsed into a `serde_json::Value` tree first (`serde_json` limits nesting to 128
//! levels, so hostile input cannot overflow the stack) and then converted by hand: Lottie
//! values come in several historical shapes (a number or a one-element array, `{"a":0,"k":…}`
//! or `{"a":1,"k":[…]}`, keyframes with an end value `e` or without, shape keyframes whose
//! values are one-element arrays of paths), which is simpler to accept explicitly than with
//! derived deserializers. Unknown keys are ignored; unknown layer and shape types become
//! [`LayerKind::Unsupported`] / [`Shape::Unsupported`] and are recorded in
//! [`Composition::unsupported`].

use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec::Vec;

use serde_json::{Map, Value};

use crate::error::LottieError;
use crate::model::{
    Animatable, Asset, Composite, Composition, DashEntry, DashKind, Ease, Ellipse, Fill, FillRule, Gradient,
    GradientFill, GradientKind, GradientStroke, Group, Keyframe, Keyframes, Layer, LayerKind, LineCap,
    LineJoin, Mask, MaskMode, PathData, PathShape, Position, Rectangle, Repeater, Rgba, Shape, Stroke,
    StrokeStyle, Transform, TrimMode, TrimPaths, Vec2,
};

/// Largest number of layers, shape items, vertices or keyframes accepted in one list (larger
/// lists are truncated): keeps hostile files from exhausting memory in the model.
const MAX_LIST: usize = 1 << 16;

/// Parses a Lottie (Bodymovin JSON) file.
///
/// Unsupported features are ignored; each one is logged once with
/// `warn!(target: "twine::lottie", "lottie: unsupported {}", what)` and listed in
/// [`Composition::unsupported`]. Invalid input returns an error, never panics.
///
/// ```
/// let json = br#"{"v":"5.7.0","fr":30,"ip":0,"op":60,"w":100,"h":100,"layers":[
///   {"ty":4,"ind":1,"ip":0,"op":60,"st":0,"ks":{},"shapes":[
///     {"ty":"el","p":{"a":0,"k":[50,50]},"s":{"a":0,"k":[40,40]}},
///     {"ty":"fl","c":{"a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100}}]}]}"#;
/// let comp = twine_lottie::load(json).unwrap();
/// assert_eq!((comp.w, comp.h, comp.fr), (100.0, 100.0, 30.0));
/// assert_eq!(comp.layers.len(), 1);
/// assert!(comp.unsupported().is_empty());
/// ```
pub fn load(json: &[u8]) -> Result<Composition, LottieError> {
    let root: Value = serde_json::from_slice(json).map_err(|e| LottieError::Json {
        line: e.line(),
        column: e.column(),
    })?;
    let Some(obj) = root.as_object() else {
        return Err(LottieError::Invalid("root is not an object"));
    };
    let mut cx = Loader::default();
    let w = num(obj.get("w")).ok_or(LottieError::Invalid("missing width `w`"))?;
    let h = num(obj.get("h")).ok_or(LottieError::Invalid("missing height `h`"))?;
    let fr = num(obj.get("fr")).ok_or(LottieError::Invalid("missing frame rate `fr`"))?;
    let ip = num(obj.get("ip")).unwrap_or(0.0);
    let op = num(obj.get("op")).ok_or(LottieError::Invalid("missing out point `op`"))?;
    if !(w > 0.0 && h > 0.0 && w.is_finite() && h.is_finite()) {
        return Err(LottieError::Invalid("size is not positive"));
    }
    if !(fr > 0.0 && fr.is_finite()) {
        return Err(LottieError::Invalid("frame rate is not positive"));
    }
    if !(op > ip && op.is_finite() && ip.is_finite()) {
        return Err(LottieError::Invalid("`op` is not after `ip`"));
    }
    let mut assets = Vec::new();
    for a in list(obj.get("assets")) {
        let Some(ao) = a.as_object() else { continue };
        let Some(layers) = ao.get("layers") else {
            if ao.contains_key("p") {
                cx.unsupported("image asset");
            }
            continue;
        };
        let id = ao.get("id").map(id_string).unwrap_or_default();
        let layers = cx.layers(Some(layers), ip, op);
        assets.push(Asset { id, layers });
    }
    let mut layers = cx.layers(obj.get("layers"), ip, op);
    // Resolve precomp references to asset indices.
    let ids: Vec<String> = assets.iter().map(|a| a.id.clone()).collect();
    for layer in layers
        .iter_mut()
        .chain(assets.iter_mut().flat_map(|a| a.layers.iter_mut()))
    {
        resolve_asset(layer, &ids, &mut cx);
    }
    if obj.get("chars").is_some_and(|c| !list(Some(c)).is_empty()) {
        cx.unsupported("text glyphs");
    }
    let mut unsupported = cx.unsupported;
    unsupported.sort();
    unsupported.dedup();
    for what in &unsupported {
        twine_core::warn!(target: "twine::lottie", "lottie: unsupported {}", what.as_str());
    }
    twine_core::debug!(
        target: "twine::lottie",
        "lottie: {}x{} @ {} fps, frames {}..{}, {} layers, {} assets",
        w,
        h,
        fr,
        ip,
        op,
        layers.len(),
        assets.len()
    );
    Ok(Composition {
        w,
        h,
        fr,
        ip,
        op,
        layers,
        assets,
        unsupported,
    })
}

fn resolve_asset(layer: &mut Layer, ids: &[String], cx: &mut Loader) {
    if let Some(r) = &layer.ref_id {
        layer.asset = ids.iter().position(|id| id == r);
        if layer.asset.is_none() {
            cx.unsupported("missing precomp asset");
        }
    }
}

/// An asset id: a string, or a number written as a string.
fn id_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => alloc::format!("{n}"),
        _ => String::new(),
    }
}

/// A JSON number (or the first element of a number array) as `f32`.
fn num(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Number(n) => n.as_f64().map(|f| f as f32).filter(|f| !f.is_nan()),
        Value::Array(a) => num(a.first()),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// A JSON number as a small integer.
fn int(v: Option<&Value>) -> Option<i64> {
    num(v).map(|f| f as i64)
}

fn list(v: Option<&Value>) -> &[Value] {
    match v {
        Some(Value::Array(a)) => &a[..a.len().min(MAX_LIST)],
        _ => &[],
    }
}

fn flag(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(v) => num(Some(v)).is_some_and(|n| n != 0.0),
        None => false,
    }
}

/// Values that can appear in a Lottie property.
trait FromJson: Sized {
    fn from_json(v: &Value) -> Option<Self>;
}

impl FromJson for f32 {
    fn from_json(v: &Value) -> Option<Self> {
        num(Some(v))
    }
}

impl FromJson for Vec2 {
    fn from_json(v: &Value) -> Option<Self> {
        match v {
            Value::Array(a) => {
                let x = num(a.first())?;
                let y = num(a.get(1)).unwrap_or(x);
                Some([x, y])
            }
            Value::Number(_) => {
                let x = num(Some(v))?;
                Some([x, x])
            }
            _ => None,
        }
    }
}

impl FromJson for Rgba {
    fn from_json(v: &Value) -> Option<Self> {
        let a = v.as_array()?;
        if a.len() < 3 {
            return None;
        }
        let c = |i: usize, d: f32| num(a.get(i)).unwrap_or(d);
        let mut rgba = [c(0, 0.0), c(1, 0.0), c(2, 0.0), c(3, 1.0)];
        // Some exporters write 0..255 channels.
        if rgba[..3].iter().any(|&x| x > 1.0) {
            for x in &mut rgba[..3] {
                *x /= 255.0;
            }
        }
        Some(rgba)
    }
}

impl FromJson for Vec<f32> {
    fn from_json(v: &Value) -> Option<Self> {
        let a = v.as_array()?;
        Some(a.iter().take(MAX_LIST).filter_map(|x| num(Some(x))).collect())
    }
}

impl FromJson for PathData {
    fn from_json(v: &Value) -> Option<Self> {
        let o = match v {
            Value::Object(o) => o,
            // Shape keyframe values are one-element arrays of paths.
            Value::Array(a) => return a.first().and_then(PathData::from_json),
            _ => return None,
        };
        let pts = |k: &str| -> Vec<Vec2> {
            list(o.get(k))
                .iter()
                .map(|p| Vec2::from_json(p).unwrap_or([0.0, 0.0]))
                .collect()
        };
        let v = pts("v");
        let mut i = pts("i");
        let mut out = pts("o");
        i.resize(v.len(), [0.0, 0.0]);
        out.resize(v.len(), [0.0, 0.0]);
        Some(PathData {
            closed: flag(o.get("c")),
            v,
            i,
            o: out,
        })
    }
}

/// Parses easing handles (`{"x": n | [n…], "y": n | [n…]}`) into per-axis points.
fn handles(v: Option<&Value>) -> Option<([Vec2; 4], usize)> {
    let o = v?.as_object()?;
    let axis = |k: &str| -> ([f32; 4], usize) {
        let mut out = [0.0; 4];
        match o.get(k) {
            Some(Value::Array(a)) if !a.is_empty() => {
                let n = a.len().min(4);
                for (i, x) in a.iter().take(4).enumerate() {
                    out[i] = num(Some(x)).unwrap_or(0.0);
                }
                for i in n..4 {
                    out[i] = out[n - 1];
                }
                (out, n)
            }
            other => {
                let x = num(other).unwrap_or(0.0);
                ([x; 4], 1)
            }
        }
    };
    let (x, nx) = axis("x");
    let (y, ny) = axis("y");
    let n = nx.max(ny);
    Some((core::array::from_fn(|i| [x[i], y[i]]), n))
}

/// Collects unsupported features while loading.
#[derive(Default)]
struct Loader {
    unsupported: Vec<String>,
}

impl Loader {
    fn unsupported(&mut self, what: &str) {
        if !self.unsupported.iter().any(|w| w == what) {
            self.unsupported.push(what.to_owned());
        }
    }

    /// An animatable property; `None` (or garbage) gives `Static(default)`.
    fn prop<T: FromJson + Clone>(&mut self, v: Option<&Value>, default: T) -> Animatable<T> {
        let Some(v) = v else {
            return Animatable::Static(default);
        };
        let Some(o) = v.as_object() else {
            return Animatable::Static(T::from_json(v).unwrap_or(default));
        };
        if o.get("x").is_some_and(Value::is_string) {
            self.unsupported("expressions");
        }
        let Some(k) = o.get("k") else {
            return Animatable::Static(default);
        };
        let keyframed = match k {
            Value::Array(a) => a
                .first()
                .and_then(Value::as_object)
                .is_some_and(|f| f.contains_key("t")),
            _ => false,
        };
        if !keyframed {
            return Animatable::Static(T::from_json(k).unwrap_or(default));
        }
        let mut frames: Vec<Keyframe<T>> = Vec::new();
        for kf in list(Some(k)) {
            let Some(kf) = kf.as_object() else { continue };
            let Some(t) = num(kf.get("t")).filter(|t| t.is_finite()) else {
                continue;
            };
            let e = kf.get("e").and_then(T::from_json);
            let s = match kf.get("s").and_then(T::from_json) {
                Some(s) => s,
                // Old files end with a keyframe that has only `t`: it holds the previous end.
                None => match frames.last() {
                    Some(p) => p.e.clone().unwrap_or_else(|| p.s.clone()),
                    None => continue,
                },
            };
            let ease = match (handles(kf.get("o")), handles(kf.get("i"))) {
                (Some((out, n1)), Some((inn, n2))) => Some(Ease {
                    out,
                    inn,
                    axes: n1.max(n2).clamp(1, 4) as u8,
                }),
                _ => None,
            };
            let tan = |key: &str| {
                kf.get(key)
                    .and_then(Vec2::from_json)
                    .filter(|p| p[0] != 0.0 || p[1] != 0.0)
            };
            frames.push(Keyframe {
                t,
                s,
                e,
                hold: flag(kf.get("h")),
                ease,
                to: tan("to"),
                ti: tan("ti"),
            });
        }
        frames.sort_by(|a, b| a.t.total_cmp(&b.t));
        match frames.len() {
            0 => Animatable::Static(default),
            1 => Animatable::Static(frames.swap_remove(0).s),
            _ => Animatable::Keyframed(Keyframes::new(frames)),
        }
    }

    fn transform(&mut self, v: Option<&Value>) -> Transform {
        let empty = Map::new();
        let o = v.and_then(Value::as_object).unwrap_or(&empty);
        let position = match o.get("p").and_then(Value::as_object) {
            Some(p) if flag(p.get("s")) => {
                Position::Split(self.prop(p.get("x"), 0.0), self.prop(p.get("y"), 0.0))
            }
            _ => Position::Combined(self.prop(o.get("p"), [0.0, 0.0])),
        };
        let rotation = if o.contains_key("r") {
            self.prop(o.get("r"), 0.0)
        } else {
            self.prop(o.get("rz"), 0.0)
        };
        if o.contains_key("rx") || o.contains_key("ry") || o.contains_key("or") {
            self.unsupported("3d rotation");
        }
        Transform {
            anchor: self.prop(o.get("a"), [0.0, 0.0]),
            position,
            scale: self.prop(o.get("s"), [100.0, 100.0]),
            rotation,
            opacity: self.prop(o.get("o"), 100.0),
            skew: self.prop(o.get("sk"), 0.0),
            skew_axis: self.prop(o.get("sa"), 0.0),
            start_opacity: self.prop(o.get("so"), 100.0),
            end_opacity: self.prop(o.get("eo"), 100.0),
        }
    }

    fn layers(&mut self, v: Option<&Value>, comp_ip: f32, comp_op: f32) -> Vec<Layer> {
        let mut layers: Vec<Layer> = list(v)
            .iter()
            .filter_map(Value::as_object)
            .map(|o| self.layer(o, comp_ip, comp_op))
            .collect();
        // Resolve parents and mattes by `ind`.
        let inds: Vec<Option<i32>> = layers.iter().map(|l| l.ind).collect();
        let find = |ind: i32| inds.iter().position(|&i| i == Some(ind));
        for (idx, layer) in layers.iter_mut().enumerate() {
            layer.parent_index = layer.parent.and_then(find).filter(|&p| p != idx);
            if layer.tt.is_some() {
                layer.matte_index = match layer.tp {
                    Some(tp) => find(tp),
                    None => idx.checked_sub(1),
                }
                .filter(|&m| m != idx);
            }
        }
        layers
    }

    fn layer(&mut self, o: &Map<String, Value>, comp_ip: f32, comp_op: f32) -> Layer {
        let ty = match int(o.get("ty")) {
            Some(0) => LayerKind::Precomp,
            Some(1) => LayerKind::Solid,
            Some(3) => LayerKind::Null,
            Some(4) => LayerKind::Shape,
            other => {
                let t = other.unwrap_or(-1).clamp(0, 255) as u8;
                self.unsupported(match t {
                    2 => "image layer",
                    5 => "text layer",
                    6 => "audio layer",
                    _ => "layer type",
                });
                LayerKind::Unsupported(t)
            }
        };
        let tt = int(o.get("tt"))
            .map(|t| t.clamp(0, 255) as u8)
            .filter(|&t| t != 0);
        match tt {
            Some(3 | 4) => self.unsupported("luma matte"),
            Some(1 | 2) | None => {}
            Some(_) => self.unsupported("matte mode"),
        }
        if int(o.get("bm")).is_some_and(|b| b != 0) {
            self.unsupported("blend mode");
        }
        if !list(o.get("ef")).is_empty() {
            self.unsupported("effects");
        }
        if flag(o.get("ao")) {
            self.unsupported("auto-orient");
        }
        if flag(o.get("ddd")) {
            self.unsupported("3d layer");
        }
        let masks = list(o.get("masksProperties"))
            .iter()
            .filter_map(Value::as_object)
            .map(|m| self.mask(m))
            .collect();
        let shapes = if ty == LayerKind::Shape {
            self.shapes(o.get("shapes"), 0).0
        } else {
            Vec::new()
        };
        let sc = o
            .get("sc")
            .and_then(Value::as_str)
            .map_or([0.0, 0.0, 0.0, 1.0], parse_hex_color);
        let sr = num(o.get("sr"))
            .filter(|s| *s > 0.0 && s.is_finite())
            .unwrap_or(1.0);
        Layer {
            ty,
            name: o.get("nm").and_then(Value::as_str).unwrap_or_default().to_owned(),
            ind: int(o.get("ind")).map(|i| i.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32),
            parent: int(o.get("parent")).map(|i| i.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32),
            parent_index: None,
            ip: num(o.get("ip")).unwrap_or(comp_ip),
            op: num(o.get("op")).unwrap_or(comp_op),
            st: num(o.get("st")).unwrap_or(0.0),
            sr,
            ks: self.transform(o.get("ks")),
            shapes,
            masks,
            tt,
            td: int(o.get("td"))
                .map(|t| t.clamp(0, 255) as u8)
                .filter(|&t| t != 0),
            tp: int(o.get("tp")).map(|i| i.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32),
            matte_index: None,
            ref_id: o.get("refId").map(id_string),
            asset: None,
            w: num(o.get("w")).unwrap_or(0.0),
            h: num(o.get("h")).unwrap_or(0.0),
            tm: o.get("tm").map(|t| self.prop(Some(t), 0.0)),
            sc,
            sw: num(o.get("sw")).unwrap_or(0.0),
            sh: num(o.get("sh")).unwrap_or(0.0),
            hidden: flag(o.get("hd")),
        }
    }

    fn mask(&mut self, o: &Map<String, Value>) -> Mask {
        let mode = match o.get("mode").and_then(Value::as_str) {
            Some("a") | None => MaskMode::Add,
            Some("s") => MaskMode::Subtract,
            Some("i") => MaskMode::Intersect,
            Some("n") => MaskMode::None,
            Some(_) => {
                self.unsupported("mask mode");
                MaskMode::Add
            }
        };
        let x = self.prop(o.get("x"), 0.0);
        if x.is_animated() || matches!(x, Animatable::Static(v) if v != 0.0) {
            self.unsupported("mask expansion");
        }
        Mask {
            mode,
            pt: self.prop(o.get("pt"), PathData::default()),
            o: self.prop(o.get("o"), 100.0),
            inv: flag(o.get("inv")),
        }
    }

    /// Shape items and the sum of their sizes. `depth` bounds the group nesting.
    fn shapes(&mut self, v: Option<&Value>, depth: u32) -> (Vec<Shape>, u32, Option<Transform>) {
        let mut items = Vec::new();
        let mut size = 0u32;
        let mut tr = None;
        for s in list(v) {
            let Some(o) = s.as_object() else { continue };
            if flag(o.get("hd")) {
                continue;
            }
            let ty = o.get("ty").and_then(Value::as_str).unwrap_or("");
            let shape = match ty {
                "gr" if depth < 64 => {
                    let (items, sz, t) = self.shapes(o.get("it"), depth + 1);
                    Shape::Group(Group {
                        items,
                        transform: t.unwrap_or_default(),
                        size: sz.saturating_add(1),
                    })
                }
                "tr" => {
                    tr = Some(self.transform(Some(s)));
                    continue;
                }
                "rc" => Shape::Rect(Rectangle {
                    p: self.prop(o.get("p"), [0.0, 0.0]),
                    s: self.prop(o.get("s"), [0.0, 0.0]),
                    r: self.prop(o.get("r"), 0.0),
                    reversed: int(o.get("d")) == Some(3),
                }),
                "el" => Shape::Ellipse(Ellipse {
                    p: self.prop(o.get("p"), [0.0, 0.0]),
                    s: self.prop(o.get("s"), [0.0, 0.0]),
                    reversed: int(o.get("d")) == Some(3),
                }),
                "sh" => Shape::Path(PathShape {
                    ks: self.prop(o.get("ks"), PathData::default()),
                    reversed: int(o.get("d")) == Some(3),
                }),
                "fl" => Shape::Fill(Fill {
                    c: self.prop(o.get("c"), [0.0, 0.0, 0.0, 1.0]),
                    o: self.prop(o.get("o"), 100.0),
                    rule: fill_rule(o),
                }),
                "st" => Shape::Stroke(Stroke {
                    c: self.prop(o.get("c"), [0.0, 0.0, 0.0, 1.0]),
                    o: self.prop(o.get("o"), 100.0),
                    style: self.stroke_style(o),
                }),
                "gf" => Shape::GradientFill(GradientFill {
                    g: self.gradient(o),
                    o: self.prop(o.get("o"), 100.0),
                    rule: fill_rule(o),
                }),
                "gs" => Shape::GradientStroke(GradientStroke {
                    g: self.gradient(o),
                    o: self.prop(o.get("o"), 100.0),
                    style: self.stroke_style(o),
                }),
                "tm" => Shape::Trim(TrimPaths {
                    s: self.prop(o.get("s"), 0.0),
                    e: self.prop(o.get("e"), 100.0),
                    o: self.prop(o.get("o"), 0.0),
                    m: if int(o.get("m")) == Some(2) {
                        TrimMode::Individually
                    } else {
                        TrimMode::Simultaneously
                    },
                }),
                "rp" => Shape::Repeater(Repeater {
                    c: self.prop(o.get("c"), 1.0),
                    o: self.prop(o.get("o"), 0.0),
                    m: if int(o.get("m")) == Some(2) {
                        Composite::Below
                    } else {
                        Composite::Above
                    },
                    tr: self.transform(o.get("tr")),
                }),
                other => {
                    let name = match other {
                        "gr" => "group nesting deeper than 64",
                        "rd" => "rounded corners",
                        "mm" => "merge paths",
                        "op" => "offset path",
                        "pb" => "pucker/bloat",
                        "tw" => "twist",
                        "zz" => "zig-zag",
                        "sr" => "polystar",
                        _ => "shape type",
                    };
                    self.unsupported(name);
                    Shape::Unsupported(other.to_owned())
                }
            };
            size = size.saturating_add(shape.size());
            items.push(shape);
        }
        (items, size, tr)
    }

    fn stroke_style(&mut self, o: &Map<String, Value>) -> StrokeStyle {
        let dash = list(o.get("d"))
            .iter()
            .filter_map(Value::as_object)
            .filter_map(|d| {
                let kind = match d.get("n").and_then(Value::as_str)? {
                    "d" => DashKind::Dash,
                    "g" => DashKind::Gap,
                    "o" => DashKind::Offset,
                    _ => return None,
                };
                Some(DashEntry {
                    kind,
                    v: self.prop(d.get("v"), 0.0),
                })
            })
            .collect();
        StrokeStyle {
            w: self.prop(o.get("w"), 1.0),
            lc: match int(o.get("lc")) {
                Some(1) => LineCap::Butt,
                Some(3) => LineCap::Square,
                _ => LineCap::Round,
            },
            lj: match int(o.get("lj")) {
                Some(1) => LineJoin::Miter,
                Some(3) => LineJoin::Bevel,
                _ => LineJoin::Round,
            },
            ml: num(o.get("ml")).filter(|m| *m >= 1.0).unwrap_or(4.0),
            dash,
        }
    }

    fn gradient(&mut self, o: &Map<String, Value>) -> Gradient {
        let g = o.get("g").and_then(Value::as_object);
        Gradient {
            kind: if int(o.get("t")) == Some(2) {
                GradientKind::Radial
            } else {
                GradientKind::Linear
            },
            s: self.prop(o.get("s"), [0.0, 0.0]),
            e: self.prop(o.get("e"), [0.0, 0.0]),
            h: self.prop(o.get("h"), 0.0),
            a: self.prop(o.get("a"), 0.0),
            points: g
                .and_then(|g| int(g.get("p")))
                .unwrap_or(0)
                .clamp(0, i64::from(u16::MAX)) as u32,
            stops: self.prop(g.and_then(|g| g.get("k")), Vec::new()),
        }
    }
}

fn fill_rule(o: &Map<String, Value>) -> FillRule {
    if int(o.get("r")) == Some(2) {
        FillRule::EvenOdd
    } else {
        FillRule::NonZero
    }
}

/// `#rrggbb` (or `#rgb`) → RGBA `0..=1` (black for anything else).
fn parse_hex_color(s: &str) -> Rgba {
    let h = s.trim().trim_start_matches('#');
    let digit = |i: usize| h.as_bytes().get(i).and_then(|c| (*c as char).to_digit(16));
    let (r, g, b) = match h.len() {
        6 => {
            let byte = |i| Some(digit(i)? * 16 + digit(i + 1)?);
            match (byte(0), byte(2), byte(4)) {
                (Some(r), Some(g), Some(b)) => (r, g, b),
                _ => (0, 0, 0),
            }
        }
        3 => match (digit(0), digit(1), digit(2)) {
            (Some(r), Some(g), Some(b)) => (r * 17, g * 17, b * 17),
            _ => (0, 0, 0),
        },
        _ => (0, 0, 0),
    };
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors() {
        assert_eq!(parse_hex_color("#ff0000"), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(parse_hex_color("#0f0"), [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(parse_hex_color("junk"), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(parse_hex_color("#zzzzzz"), [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn number_forms() {
        let v: Value = serde_json::from_str("[3]").unwrap();
        assert_eq!(num(Some(&v)), Some(3.0));
        let v: Value = serde_json::from_str("5").unwrap();
        assert_eq!(Vec2::from_json(&v), Some([5.0, 5.0]));
        let v: Value = serde_json::from_str("[1, 2, 0]").unwrap();
        assert_eq!(Vec2::from_json(&v), Some([1.0, 2.0]));
    }
}
