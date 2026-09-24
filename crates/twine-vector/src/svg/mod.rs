//! SVG Tiny subset parser (feature `svg`): [`parse_svg`] turns SVG bytes into an
//! [`SvgDocument`] holding a [`VectorScene`], drawn scaled into any rectangle with
//! [`SvgDocument::render`].
//!
//! Supported:
//!
//! - elements `svg` (`width`, `height`, `viewBox`, `preserveAspectRatio`), `g`, `path` (the
//!   full `d` grammar), `rect` (`rx`/`ry`), `circle`, `ellipse`, `line`, `polyline`,
//!   `polygon`, and `linearGradient` / `radialGradient` with `stop`s (anywhere, usually in
//!   `defs`), referenced as `url(#id)`;
//! - presentation attributes and inline `style="…"` declarations: `fill`, `stroke`,
//!   `stroke-width`, `stroke-linejoin`, `stroke-linecap`, `stroke-miterlimit`,
//!   `stroke-dasharray`, `stroke-dashoffset`, `opacity`, `fill-opacity`, `stroke-opacity`,
//!   `fill-rule`, `color` (for `currentColor`), `display="none"`, and `transform`
//!   (`matrix`, `translate`, `scale`, `rotate`, `skewX`, `skewY`);
//! - gradients with `gradientUnits`, `gradientTransform`, `spreadMethod`, `x1 y1 x2 y2`,
//!   `cx cy r fx fy`, `stop` `offset`/`stop-color`/`stop-opacity` and `href` / `xlink:href`
//!   inheritance;
//! - colors `#rgb`, `#rrggbb`, `rgb()`, the 147 color keywords, `none`, `currentColor`.
//!
//! Everything else (text, `use`, clipping, masks, filters, CSS style sheets, …) is ignored
//! with a `debug!` log. Group `opacity` is multiplied into the elements (no offscreen layer).
//!
//! The parser is iterative (element nesting is limited to [`MAX_DEPTH`]), never panics, and
//! allocates only per element (paths, gradient stops).
//!
//! ```
//! use twine_vector::parse_svg;
//! let doc = parse_svg(br#"<svg viewBox="0 0 24 24"><path d="M12 2 22 22H2z" fill="red"/></svg>"#).unwrap();
//! assert_eq!(doc.scene.len(), 1);
//! assert_eq!(doc.view_box.width(), twine_core::Fx::from_int(24));
//! ```

mod color;
mod number;
mod path_data;
mod xml;

use alloc::string::String;
use alloc::vec::Vec;

use twine_core::{Color, Fx, Opa, Rect, Transform};
use twine_render::{FillRule, GradExtend, GradStop, Painter};

use self::color::parse_color;
use self::number::{Cursor, Length, deg_to_angle, length, number};
use self::path_data::{parse_list, parse_path_data, parse_points};
use self::xml::{Attrs, Reader, Token, decode, unescape};
use crate::draw::VectorDsc;
use twine_core::math::isqrt64;

use crate::geom::{FxPoint, FxRect, FxSize};
use crate::paint::{Paint, Stops};
use crate::path::Path;
use crate::scene::VectorScene;
use crate::stroke::{Dash, LineCap, LineJoin, MAX_DASHES, Stroke};

// NOTE(P23.S08): the engine's `ImageSource::Svg` parses once into an `SvgDocument` (cached) and
// draws it with `render`, composing the image widget's scale/rotation into `transform_for`.

/// Deepest element nesting accepted.
pub const MAX_DEPTH: usize = 32;

/// Longest `href` chain followed between gradients.
const MAX_HREF_CHAIN: usize = 8;

/// Why an SVG document could not be parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, thiserror::Error)]
pub enum SvgError {
    /// The input is not UTF-8.
    #[error("svg: input is not valid UTF-8")]
    InvalidUtf8,
    /// Malformed markup at this byte offset.
    #[error("svg: malformed XML at byte {0}")]
    Malformed(usize),
    /// The input ended inside a tag, comment or element.
    #[error("svg: unexpected end of input")]
    UnexpectedEof,
    /// Elements nested deeper than [`MAX_DEPTH`].
    #[error("svg: elements nested deeper than {MAX_DEPTH}")]
    TooDeep,
    /// The root element is not `<svg>`.
    #[error("svg: the root element is not <svg>")]
    NotSvg,
}

/// `preserveAspectRatio` alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Align {
    /// Scale non-uniformly to fill the viewport.
    None,
    /// Align min x / min y.
    XMinYMin,
    /// Align mid x / min y.
    XMidYMin,
    /// Align max x / min y.
    XMaxYMin,
    /// Align min x / mid y.
    XMinYMid,
    /// Center (the default).
    #[default]
    XMidYMid,
    /// Align max x / mid y.
    XMaxYMid,
    /// Align min x / max y.
    XMinYMax,
    /// Align mid x / max y.
    XMidYMax,
    /// Align max x / max y.
    XMaxYMax,
}

/// `preserveAspectRatio`: alignment and `meet` (fit inside, the default) or `slice` (cover).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AspectRatio {
    /// Alignment.
    pub align: Align,
    /// `slice` instead of `meet`.
    pub slice: bool,
}

/// A parsed SVG document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SvgDocument {
    /// Intrinsic size (`width`/`height`, else the `viewBox` size, else 100 × 100).
    pub size: FxSize,
    /// The `viewBox` (the user-space rectangle mapped onto the drawing area).
    pub view_box: FxRect,
    /// How the view box is fitted into the drawing area.
    pub aspect: AspectRatio,
    /// The shapes, in user space.
    pub scene: VectorScene,
}

impl SvgDocument {
    /// The transform that maps the view box into `dst` (screen coordinates) according to
    /// [`aspect`](Self::aspect).
    #[must_use]
    pub fn transform_for(&self, dst: Rect) -> Transform {
        let vb = self.view_box;
        let (vw, vh) = (i64::from(vb.width().0), i64::from(vb.height().0));
        if vw <= 0 || vh <= 0 || dst.is_empty() {
            return Transform::translate(Fx::from_int(dst.x0), Fx::from_int(dst.y0));
        }
        let (dw, dh) = (i64::from(dst.width()) << 16, i64::from(dst.height()) << 16);
        let sx = ((dw << 16) / vw).clamp(1, i64::from(i32::MAX)) as i32;
        let sy = ((dh << 16) / vh).clamp(1, i64::from(i32::MAX)) as i32;
        let (sx, sy) = match (self.aspect.align, self.aspect.slice) {
            (Align::None, _) => (sx, sy),
            (_, false) => (sx.min(sy), sx.min(sy)),
            (_, true) => (sx.max(sy), sx.max(sy)),
        };
        // Free space, split per the alignment (0, ½ or 1).
        let (fx, fy) = match self.aspect.align {
            Align::None | Align::XMinYMin => (0, 0),
            Align::XMidYMin => (1, 0),
            Align::XMaxYMin => (2, 0),
            Align::XMinYMid => (0, 1),
            Align::XMidYMid => (1, 1),
            Align::XMaxYMid => (2, 1),
            Align::XMinYMax => (0, 2),
            Align::XMidYMax => (1, 2),
            Align::XMaxYMax => (2, 2),
        };
        let free_x = dw - ((vw * i64::from(sx)) >> 16);
        let free_y = dh - ((vh * i64::from(sy)) >> 16);
        let tx = (i64::from(dst.x0) << 16) + free_x * fx / 2 - ((i64::from(vb.x0.0) * i64::from(sx)) >> 16);
        let ty = (i64::from(dst.y0) << 16) + free_y * fy / 2 - ((i64::from(vb.y0.0) * i64::from(sy)) >> 16);
        Transform {
            a: Fx(sx),
            b: Fx::ZERO,
            c: Fx::ZERO,
            d: Fx(sy),
            tx: Fx(crate::geom::sat(tx)),
            ty: Fx(crate::geom::sat(ty)),
        }
    }

    /// Draws the document scaled into `dst` (clipped to `dst`).
    pub fn render(&self, painter: &mut Painter<'_>, dst: Rect) {
        let t = self.transform_for(dst);
        painter.with_clip(dst, |p| self.scene.draw(p, &t));
    }
}

/// Parses an SVG document.
///
/// Returns an error only for markup that cannot be tokenized (malformed tags, unterminated
/// comments, nesting deeper than [`MAX_DEPTH`], invalid UTF-8) or a non-`svg` root; invalid
/// attribute values are ignored (logged), and path data is kept up to its first error.
pub fn parse_svg(bytes: &[u8]) -> Result<SvgDocument, SvgError> {
    let src = core::str::from_utf8(bytes).map_err(|_| SvgError::InvalidUtf8)?;
    let grads = collect_gradients(src)?;
    let mut p = Parser {
        grads: &grads,
        buf: String::new(),
        vp: (Fx::from_int(100), Fx::from_int(100)),
    };
    p.build(src)
}

// ---------------------------------------------------------------- style

/// A fill or stroke paint as specified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaintSpec {
    None,
    Color(Color),
    Current,
    /// Gradient index, fallback color.
    Grad(usize, Option<Color>),
}

/// Inherited presentation state.
#[derive(Clone, Debug)]
struct Style {
    fill: PaintSpec,
    stroke: PaintSpec,
    fill_opa: Opa,
    stroke_opa: Opa,
    /// Product of the `opacity` of this element and its ancestors.
    group_opa: Opa,
    stroke_width: Length,
    join: LineJoin,
    cap: LineCap,
    miter: Fx,
    dash: Option<heapless::Vec<Fx, MAX_DASHES>>,
    dash_offset: Fx,
    fill_rule: FillRule,
    color: Color,
    display: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fill: PaintSpec::Color(Color::BLACK),
            stroke: PaintSpec::None,
            fill_opa: Opa::COVER,
            stroke_opa: Opa::COVER,
            group_opa: Opa::COVER,
            stroke_width: Length::Px(Fx::ONE),
            join: LineJoin::Miter,
            cap: LineCap::Butt,
            miter: Fx::from_int(4),
            dash: None,
            dash_offset: Fx::ZERO,
            fill_rule: FillRule::NonZero,
            color: Color::BLACK,
            display: true,
        }
    }
}

/// An opacity value `0..1` (or a percentage) as [`Opa`].
fn parse_opacity(v: &str) -> Option<Opa> {
    let l = length(v)?;
    let raw = match l {
        Length::Px(f) => i64::from(f.0),
        Length::Pct(p) => i64::from(p.0) / 100,
    };
    Some(Opa(((raw.clamp(0, 65_536) * 255 + 32_768) >> 16) as u8))
}

// ---------------------------------------------------------------- gradients

/// A gradient element as written (unset attributes are inherited through `href`).
#[derive(Clone, Debug, Default)]
struct GradDef<'a> {
    id: &'a str,
    radial: bool,
    href: Option<&'a str>,
    user_space: Option<bool>,
    transform: Option<Transform>,
    spread: Option<GradExtend>,
    /// x1 y1 x2 y2 (linear) or cx cy r fx fy (radial).
    coords: [Option<Length>; 5],
    stops: Vec<GradStop>,
}

const LINEAR_ATTRS: [&str; 4] = ["x1", "y1", "x2", "y2"];
const RADIAL_ATTRS: [&str; 5] = ["cx", "cy", "r", "fx", "fy"];

/// First pass: every `linearGradient` / `radialGradient` with its stops.
fn collect_gradients(src: &str) -> Result<Vec<GradDef<'_>>, SvgError> {
    let mut r = Reader::new(src);
    let mut out: Vec<GradDef<'_>> = Vec::new();
    let mut depth = 0usize;
    // Depth at which the gradient being filled was opened.
    let mut open: Option<(usize, usize)> = None;
    let mut buf = String::new();
    while let Some(t) = r.next_token()? {
        match t {
            Token::Start {
                name,
                attrs,
                self_closing,
            } => {
                if depth >= MAX_DEPTH {
                    return Err(SvgError::TooDeep);
                }
                match local(name) {
                    "linearGradient" | "radialGradient" => {
                        let g = parse_grad_def(name, attrs, &mut buf);
                        out.push(g);
                        if !self_closing {
                            open = Some((out.len() - 1, depth));
                        }
                    }
                    "stop" => {
                        if let Some((gi, d)) = open {
                            if d + 1 == depth {
                                let s = parse_stop(attrs, &mut buf, out[gi].stops.last().map(|s| s.frac));
                                out[gi].stops.push(s);
                            }
                        }
                    }
                    _ => {}
                }
                if !self_closing {
                    depth += 1;
                }
            }
            Token::End { .. } => {
                depth = depth.saturating_sub(1);
                if open.is_some_and(|(_, d)| d == depth) {
                    open = None;
                }
            }
        }
    }
    Ok(out)
}

/// The element name without a namespace prefix.
fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn parse_grad_def<'a>(name: &str, attrs: Attrs<'a>, buf: &mut String) -> GradDef<'a> {
    let radial = local(name) == "radialGradient";
    let mut g = GradDef {
        radial,
        ..GradDef::default()
    };
    let keys: &[&str] = if radial { &RADIAL_ATTRS } else { &LINEAR_ATTRS };
    for (k, raw) in attrs.iter() {
        let v = unescape(raw, buf);
        match k {
            "id" => g.id = raw,
            "href" | "xlink:href" => g.href = raw.strip_prefix('#'),
            "gradientUnits" => {
                g.user_space = match v.trim() {
                    "userSpaceOnUse" => Some(true),
                    "objectBoundingBox" => Some(false),
                    _ => None,
                };
            }
            "gradientTransform" => g.transform = parse_transform(v),
            "spreadMethod" => {
                g.spread = match v.trim() {
                    "pad" => Some(GradExtend::Pad),
                    "reflect" => Some(GradExtend::Reflect),
                    "repeat" => Some(GradExtend::Repeat),
                    _ => None,
                };
            }
            _ => {
                if let Some(i) = keys.iter().position(|a| *a == k) {
                    g.coords[i] = length(v);
                }
            }
        }
    }
    g
}

fn parse_stop(attrs: Attrs<'_>, buf: &mut String, prev: Option<u8>) -> GradStop {
    let mut offset = Fx::ZERO;
    let mut color = Color::BLACK;
    let mut opa = Opa::COVER;
    let mut apply = |k: &str, v: &str| match k {
        "offset" => {
            if let Some(l) = length(v) {
                offset = match l {
                    Length::Px(f) => f,
                    Length::Pct(p) => Fx(p.0 / 100),
                };
            }
        }
        "stop-color" => {
            if let Some(c) = parse_color(v) {
                color = c;
            }
        }
        "stop-opacity" => {
            if let Some(o) = parse_opacity(v) {
                opa = o;
            }
        }
        _ => {}
    };
    let mut style: Option<&str> = None;
    for (k, raw) in attrs.iter() {
        if k == "style" {
            style = Some(raw);
        } else {
            let v = unescape(raw, buf);
            apply(k, v);
        }
    }
    if let Some(s) = style {
        for (k, v) in declarations(s) {
            apply(k, v);
        }
    }
    let frac = ((i64::from(offset.0).clamp(0, 65_536) * 255 + 32_768) >> 16) as u8;
    GradStop::with_opa(color, opa, frac.max(prev.unwrap_or(0)))
}

/// A gradient with `href` inheritance resolved.
struct Resolved<'g> {
    radial: bool,
    user_space: bool,
    transform: Transform,
    spread: GradExtend,
    coords: [Option<Length>; 5],
    stops: &'g [GradStop],
}

fn resolve_grad<'g>(grads: &'g [GradDef<'_>], i: usize) -> Resolved<'g> {
    let g = &grads[i];
    let mut r = Resolved {
        radial: g.radial,
        user_space: g.user_space.unwrap_or(false),
        transform: g.transform.unwrap_or(Transform::IDENTITY),
        spread: g.spread.unwrap_or_default(),
        coords: g.coords,
        stops: &g.stops,
    };
    let (mut us, mut tr, mut sp) = (g.user_space, g.transform, g.spread);
    let mut cur = g;
    for _ in 0..MAX_HREF_CHAIN {
        let Some(h) = cur.href else { break };
        let Some(next) = grads.iter().find(|x| x.id == h) else {
            break;
        };
        if r.stops.is_empty() {
            r.stops = &next.stops;
        }
        us = us.or(next.user_space);
        tr = tr.or(next.transform);
        sp = sp.or(next.spread);
        // Geometry attributes are inherited between gradients of the same kind.
        if next.radial == g.radial {
            for (c, n) in r.coords.iter_mut().zip(next.coords) {
                if c.is_none() {
                    *c = n;
                }
            }
        }
        cur = next;
    }
    r.user_space = us.unwrap_or(false);
    r.transform = tr.unwrap_or(Transform::IDENTITY);
    r.spread = sp.unwrap_or_default();
    r
}

// ---------------------------------------------------------------- transforms

/// Parses a transform list (`None` when invalid: the attribute is then ignored).
fn parse_transform(s: &str) -> Option<Transform> {
    let mut c = Cursor::new(s);
    let mut acc = Transform::IDENTITY;
    loop {
        c.skip_sep();
        if c.at_end() {
            return Some(acc);
        }
        let name = c.word();
        if !c.eat(b'(') {
            return None;
        }
        let mut args = [Fx::ZERO; 6];
        let mut n = 0;
        while !c.eat(b')') {
            if n == 6 {
                return None;
            }
            args[n] = c.number()?;
            c.skip_sep();
            n += 1;
        }
        let t = match (name, n) {
            (b"matrix", 6) => Transform {
                a: args[0],
                b: args[1],
                c: args[2],
                d: args[3],
                tx: args[4],
                ty: args[5],
            },
            (b"translate", 1) => Transform::translate(args[0], Fx::ZERO),
            (b"translate", 2) => Transform::translate(args[0], args[1]),
            (b"scale", 1) => Transform::scale(args[0], args[0]),
            (b"scale", 2) => Transform::scale(args[0], args[1]),
            (b"rotate", 1) => Transform::rotate(deg_to_angle(args[0])),
            (b"rotate", 3) => {
                let (cx, cy) = (args[1], args[2]);
                Transform::translate(-cx, -cy)
                    .then(Transform::rotate(deg_to_angle(args[0])))
                    .then(Transform::translate(cx, cy))
            }
            (b"skewX", 1) => Transform::skew(deg_to_angle(args[0]), twine_core::Angle(0)),
            (b"skewY", 1) => Transform::skew(twine_core::Angle(0), deg_to_angle(args[0])),
            _ => return None,
        };
        // "A B" applies B first, then A.
        acc = t.then(acc);
    }
}

// ---------------------------------------------------------------- style declarations

/// Iterates `name: value` declarations of a `style` attribute.
fn declarations(s: &str) -> impl Iterator<Item = (&str, &str)> {
    s.split(';').filter_map(|d| {
        let (k, v) = d.split_once(':')?;
        let v = v.trim();
        let v = v.strip_suffix("!important").map_or(v, str::trim);
        Some((k.trim(), v))
    })
}

// ---------------------------------------------------------------- builder

struct Parser<'g, 'a> {
    grads: &'g [GradDef<'a>],
    buf: String,
    /// Viewport size (for percentages).
    vp: (Fx, Fx),
}

/// One open element.
struct Frame<'a> {
    name: &'a str,
    style: Style,
    ctm: Transform,
    skip: bool,
}

impl Parser<'_, '_> {
    fn vp_diag(&self) -> Fx {
        let (w, h) = (i64::from(self.vp.0.0), i64::from(self.vp.1.0));
        // √((w² + h²) / 2); w, h < 2^31, so the sum of squares fits in u64.
        let s = u64::midpoint(w.unsigned_abs().pow(2), h.unsigned_abs().pow(2));
        Fx(isqrt64(s) as i32)
    }

    fn len_x(&self, v: Option<&str>, default: Fx) -> Fx {
        v.and_then(length).map_or(default, |l| l.resolve(self.vp.0))
    }

    fn len_y(&self, v: Option<&str>, default: Fx) -> Fx {
        v.and_then(length).map_or(default, |l| l.resolve(self.vp.1))
    }

    fn len_d(&self, v: Option<&str>, default: Fx) -> Fx {
        v.and_then(length).map_or(default, |l| l.resolve(self.vp_diag()))
    }

    fn paint_spec(&self, v: &str, parent: PaintSpec) -> PaintSpec {
        let v = v.trim();
        match v {
            "none" => return PaintSpec::None,
            "currentColor" => return PaintSpec::Current,
            "inherit" => return parent,
            _ => {}
        }
        if let Some(rest) = v.strip_prefix("url(") {
            let Some((inner, after)) = rest.split_once(')') else {
                return parent;
            };
            let id = inner.trim().trim_matches(|c| c == '\'' || c == '"');
            let id = id.strip_prefix('#').unwrap_or(id);
            let fallback = match after.trim() {
                "" | "none" => None,
                f => parse_color(f),
            };
            return match self.grads.iter().position(|g| g.id == id) {
                Some(i) => PaintSpec::Grad(i, fallback),
                None => fallback.map_or(PaintSpec::None, PaintSpec::Color),
            };
        }
        parse_color(v).map_or(parent, PaintSpec::Color)
    }

    /// Applies one presentation property.
    fn apply(&self, st: &mut Style, parent: &Style, k: &str, v: &str) {
        match k {
            "fill" => st.fill = self.paint_spec(v, parent.fill),
            "stroke" => st.stroke = self.paint_spec(v, parent.stroke),
            "fill-opacity" => st.fill_opa = parse_opacity(v).unwrap_or(st.fill_opa),
            "stroke-opacity" => st.stroke_opa = parse_opacity(v).unwrap_or(st.stroke_opa),
            "opacity" => {
                if let Some(o) = parse_opacity(v) {
                    st.group_opa = parent.group_opa.mul(o);
                }
            }
            "stroke-width" => {
                if let Some(l) = length(v).filter(|l| !matches!(l, Length::Px(x) | Length::Pct(x) if x.0 < 0))
                {
                    st.stroke_width = l;
                }
            }
            "stroke-linejoin" => {
                st.join = match v.trim() {
                    "miter" | "miter-clip" | "arcs" => LineJoin::Miter,
                    "round" => LineJoin::Round,
                    "bevel" => LineJoin::Bevel,
                    _ => st.join,
                };
            }
            "stroke-linecap" => {
                st.cap = match v.trim() {
                    "butt" => LineCap::Butt,
                    "round" => LineCap::Round,
                    "square" => LineCap::Square,
                    _ => st.cap,
                };
            }
            "stroke-miterlimit" => {
                if let Some(m) = number(v).filter(|m| *m >= Fx::ONE) {
                    st.miter = m;
                }
            }
            "stroke-dasharray" => {
                if v.trim() == "none" {
                    st.dash = None;
                } else {
                    let mut list = heapless::Vec::new();
                    if parse_list(v, &mut list).is_some() {
                        let valid = list.iter().all(|x| x.0 >= 0) && list.iter().any(|x| x.0 > 0);
                        st.dash = valid.then_some(list);
                    }
                }
            }
            "stroke-dashoffset" => st.dash_offset = number(v).unwrap_or(st.dash_offset),
            "fill-rule" => {
                st.fill_rule = match v.trim() {
                    "evenodd" => FillRule::EvenOdd,
                    "nonzero" => FillRule::NonZero,
                    _ => st.fill_rule,
                };
            }
            "color" => st.color = parse_color(v).unwrap_or(st.color),
            "display" => st.display = v.trim() != "none",
            _ => {}
        }
    }

    /// The element's style: inherited from `parent`, then attributes, then `style="…"`.
    fn style_of(&mut self, attrs: Attrs<'_>, parent: &Style) -> Style {
        let mut st = parent.clone();
        // `display` is not inherited, but a hidden parent skips its subtree anyway.
        st.display = true;
        let mut inline: Option<&str> = None;
        let mut buf = core::mem::take(&mut self.buf);
        for (k, raw) in attrs.iter() {
            if k == "style" {
                inline = Some(raw);
                continue;
            }
            let v = unescape(raw, &mut buf);
            self.apply(&mut st, parent, k, v);
        }
        if let Some(s) = inline {
            let s = unescape(s, &mut buf);
            for (k, v) in declarations(s) {
                self.apply(&mut st, parent, k, v);
            }
        }
        self.buf = buf;
        st
    }

    fn attr_transform(attrs: Attrs<'_>) -> Transform {
        match attrs.get("transform") {
            Some(t) => parse_transform(t).unwrap_or_else(|| {
                twine_core::debug!(target: "twine::vector", "svg: invalid transform ignored");
                Transform::IDENTITY
            }),
            None => Transform::IDENTITY,
        }
    }

    fn build(&mut self, src: &str) -> Result<SvgDocument, SvgError> {
        let mut r = Reader::new(src);
        let mut stack: Vec<Frame<'_>> = Vec::with_capacity(8);
        let mut doc: Option<SvgDocument> = None;
        let mut root_done = false;
        while let Some(t) = r.next_token()? {
            match t {
                Token::End { name } => {
                    match stack.pop() {
                        Some(f) if f.name == name => {}
                        _ => return Err(SvgError::Malformed(0)),
                    }
                    if stack.is_empty() {
                        root_done = true;
                    }
                }
                Token::Start {
                    name,
                    attrs,
                    self_closing,
                } => {
                    if root_done {
                        return Err(SvgError::Malformed(0));
                    }
                    if stack.len() >= MAX_DEPTH {
                        return Err(SvgError::TooDeep);
                    }
                    let frame = match stack.last() {
                        None => {
                            if local(name) != "svg" {
                                return Err(SvgError::NotSvg);
                            }
                            let d = self.root(attrs);
                            let style = self.style_of(attrs, &Style::default());
                            doc = Some(d);
                            Frame {
                                name,
                                style,
                                ctm: Transform::IDENTITY,
                                skip: false,
                            }
                        }
                        Some(parent) if parent.skip => Frame {
                            name,
                            style: Style::default(),
                            ctm: Transform::IDENTITY,
                            skip: true,
                        },
                        Some(parent) => {
                            let (pstyle, pctm) = (parent.style.clone(), parent.ctm);
                            let style = self.style_of(attrs, &pstyle);
                            let ctm = Self::attr_transform(attrs).then(pctm);
                            let skip =
                                !style.display || !self.element(name, attrs, &style, &ctm, doc.as_mut());
                            Frame {
                                name,
                                style,
                                ctm,
                                skip,
                            }
                        }
                    };
                    if self_closing {
                        if stack.is_empty() {
                            root_done = true;
                        }
                    } else {
                        stack.push(frame);
                    }
                }
            }
        }
        if !stack.is_empty() {
            return Err(SvgError::UnexpectedEof);
        }
        doc.ok_or(SvgError::NotSvg)
    }

    /// Reads the root `svg` element's geometry.
    fn root(&mut self, attrs: Attrs<'_>) -> SvgDocument {
        let mut vb: Option<FxRect> = None;
        if let Some(v) = attrs.get("viewBox") {
            let mut c = Cursor::new(v);
            let vals: Option<[Fx; 4]> =
                (|| Some([c.number_sep()?, c.number_sep()?, c.number_sep()?, c.number_sep()?]))();
            if let Some([x, y, w, h]) = vals {
                if w.0 > 0 && h.0 > 0 {
                    vb = Some(FxRect::from_xywh(x, y, w, h));
                }
            }
        }
        let abs = |v: Option<&str>| match v.and_then(length) {
            Some(Length::Px(p)) if p.0 > 0 => Some(p),
            _ => None,
        };
        let (w, h) = (abs(attrs.get("width")), abs(attrs.get("height")));
        let hundred = Fx::from_int(100);
        let size = FxSize::new(
            w.or(vb.map(|b| b.width())).unwrap_or(hundred),
            h.or(vb.map(|b| b.height())).unwrap_or(hundred),
        );
        let view_box = vb.unwrap_or(FxRect::from_xywh(Fx::ZERO, Fx::ZERO, size.w, size.h));
        self.vp = (view_box.width(), view_box.height());
        let mut aspect = AspectRatio::default();
        if let Some(v) = attrs.get("preserveAspectRatio") {
            let mut it = v.split_ascii_whitespace();
            if let Some(a) = it.next() {
                aspect.align = match a {
                    "none" => Align::None,
                    "xMinYMin" => Align::XMinYMin,
                    "xMidYMin" => Align::XMidYMin,
                    "xMaxYMin" => Align::XMaxYMin,
                    "xMinYMid" => Align::XMinYMid,
                    "xMaxYMid" => Align::XMaxYMid,
                    "xMinYMax" => Align::XMinYMax,
                    "xMidYMax" => Align::XMidYMax,
                    "xMaxYMax" => Align::XMaxYMax,
                    _ => Align::XMidYMid,
                };
            }
            aspect.slice = it.next() == Some("slice");
        }
        SvgDocument {
            size,
            view_box,
            aspect,
            scene: VectorScene::new(),
        }
    }

    /// Handles a non-root element. Returns `false` when its subtree is to be skipped.
    fn element(
        &mut self,
        name: &str,
        attrs: Attrs<'_>,
        st: &Style,
        ctm: &Transform,
        doc: Option<&mut SvgDocument>,
    ) -> bool {
        let Some(doc) = doc else { return false };
        let get = |k: &str| attrs.get(k).map(decode);
        let mut path = Path::new();
        match local(name) {
            "g" => return true,
            "svg" => {
                twine_core::debug!(target: "twine::vector", "svg: nested <svg> drawn as a group");
                return true;
            }
            "path" => {
                if let Some(d) = get("d") {
                    if !parse_path_data(&d, &mut path) {
                        twine_core::debug!(target: "twine::vector", "svg: path data error; drawn up to the error");
                    }
                }
            }
            "rect" => {
                let x = self.len_x(get("x").as_deref(), Fx::ZERO);
                let y = self.len_y(get("y").as_deref(), Fx::ZERO);
                let w = self.len_x(get("width").as_deref(), Fx::ZERO);
                let h = self.len_y(get("height").as_deref(), Fx::ZERO);
                let rx = get("rx")
                    .as_deref()
                    .and_then(length)
                    .map(|l| l.resolve(self.vp.0));
                let ry = get("ry")
                    .as_deref()
                    .and_then(length)
                    .map(|l| l.resolve(self.vp.1));
                let (rx, ry) = match (rx, ry) {
                    (Some(a), Some(b)) => (a, b),
                    (Some(a), None) => (a, a),
                    (None, Some(b)) => (b, b),
                    (None, None) => (Fx::ZERO, Fx::ZERO),
                };
                if w.0 > 0 && h.0 > 0 {
                    path.rounded_rect(FxRect::from_xywh(x, y, w, h), rx, ry);
                }
            }
            "circle" => {
                let c = FxPoint::new(
                    self.len_x(get("cx").as_deref(), Fx::ZERO),
                    self.len_y(get("cy").as_deref(), Fx::ZERO),
                );
                let r = self.len_d(get("r").as_deref(), Fx::ZERO);
                path.circle(c, r);
            }
            "ellipse" => {
                let c = FxPoint::new(
                    self.len_x(get("cx").as_deref(), Fx::ZERO),
                    self.len_y(get("cy").as_deref(), Fx::ZERO),
                );
                let rx = self.len_x(get("rx").as_deref(), Fx::ZERO);
                let ry = self.len_y(get("ry").as_deref(), Fx::ZERO);
                path.ellipse(c, rx, ry);
            }
            "line" => {
                let a = FxPoint::new(
                    self.len_x(get("x1").as_deref(), Fx::ZERO),
                    self.len_y(get("y1").as_deref(), Fx::ZERO),
                );
                let b = FxPoint::new(
                    self.len_x(get("x2").as_deref(), Fx::ZERO),
                    self.len_y(get("y2").as_deref(), Fx::ZERO),
                );
                path.move_to(a).line_to(b);
            }
            "polyline" | "polygon" => {
                if let Some(p) = get("points") {
                    parse_points(&p, &mut path, local(name) == "polygon");
                }
            }
            "linearGradient" | "radialGradient" | "defs" => return false,
            other => {
                twine_core::debug!(target: "twine::vector", "svg: unsupported <{}>", other);
                return false;
            }
        }
        if !path.is_empty() {
            let is_line = local(name) == "line";
            if let Some(dsc) = self.dsc(st, ctm, &path, is_line) {
                doc.scene.add(path, dsc);
            }
        }
        // Shapes have no rendered children.
        false
    }

    /// The paint of `spec` for a shape with user-space bounds `bb`; the second value is an
    /// extra opacity (a one-stop gradient is a solid color with that stop's opacity).
    fn paint(&self, spec: PaintSpec, st: &Style, bb: FxRect) -> Option<(Paint, Opa)> {
        match spec {
            PaintSpec::None => None,
            PaintSpec::Color(c) => Some((Paint::Solid(c), Opa::COVER)),
            PaintSpec::Current => Some((Paint::Solid(st.color), Opa::COVER)),
            PaintSpec::Grad(i, fallback) => {
                let g = resolve_grad(self.grads, i);
                match g.stops {
                    [] => return fallback.map(|c| (Paint::Solid(c), Opa::COVER)),
                    [s] => return Some((Paint::Solid(s.color), s.opa)),
                    _ => {}
                }
                let base = if g.user_space {
                    Transform::IDENTITY
                } else {
                    if bb.width().0 <= 0 || bb.height().0 <= 0 {
                        return None;
                    }
                    Transform::scale(bb.width(), bb.height()).then(Transform::translate(bb.x0, bb.y0))
                };
                let transform = g.transform.then(base);
                // Coordinates: fractions of the bounding box, or user units (percentages of
                // the viewport).
                let res = |l: Option<Length>, def_pct: i32, base: Fx| -> Fx {
                    let l = l.unwrap_or(Length::Pct(Fx::from_int(def_pct)));
                    if g.user_space {
                        l.resolve(base)
                    } else {
                        match l {
                            Length::Px(v) => v,
                            Length::Pct(p) => Fx(p.0 / 100),
                        }
                    }
                };
                let stops = Stops::Owned(g.stops.to_vec());
                let (w, h, d) = (self.vp.0, self.vp.1, self.vp_diag());
                let paint = if g.radial {
                    let cx = res(g.coords[0], 50, w);
                    let cy = res(g.coords[1], 50, h);
                    let r = res(g.coords[2], 50, d);
                    let fx = g.coords[3].map_or(cx, |l| res(Some(l), 50, w));
                    let fy = g.coords[4].map_or(cy, |l| res(Some(l), 50, h));
                    let c = FxPoint::new(cx, cy);
                    let f = FxPoint::new(fx, fy);
                    if r.0 <= 0 {
                        // Zero radius: the last stop color.
                        let s = g.stops[g.stops.len() - 1];
                        return Some((Paint::Solid(s.color), s.opa));
                    }
                    Paint::Radial {
                        center: c,
                        radius: r,
                        focal: (f != c).then_some(f),
                        stops,
                        extend: g.spread,
                        transform,
                    }
                } else {
                    Paint::Linear {
                        start: FxPoint::new(res(g.coords[0], 0, w), res(g.coords[1], 0, h)),
                        end: FxPoint::new(res(g.coords[2], 100, w), res(g.coords[3], 0, h)),
                        stops,
                        extend: g.spread,
                        transform,
                    }
                };
                Some((paint, Opa::COVER))
            }
        }
    }

    fn dsc(&self, st: &Style, ctm: &Transform, path: &Path, is_line: bool) -> Option<VectorDsc> {
        let bb = path.bounds();
        let fill = if is_line {
            None
        } else {
            self.paint(st.fill, st, bb)
        };
        let stroke_w = st.stroke_width.resolve(self.vp_diag());
        let stroke = if stroke_w.0 > 0 {
            self.paint(st.stroke, st, bb)
        } else {
            None
        };
        if fill.is_none() && stroke.is_none() {
            return None;
        }
        let (fill, fill_extra) = match fill {
            Some((p, o)) => (Some((p, st.fill_rule)), o),
            None => (None, Opa::COVER),
        };
        let (stroke, stroke_extra) = match stroke {
            Some((p, o)) => (
                Some((
                    p,
                    Stroke {
                        width: stroke_w,
                        join: st.join,
                        cap: st.cap,
                        miter_limit: st.miter,
                        dash: st.dash.as_ref().map(|d| Dash {
                            pattern: d.clone(),
                            offset: st.dash_offset,
                        }),
                    },
                )),
                o,
            ),
            None => (None, Opa::COVER),
        };
        Some(VectorDsc {
            transform: *ctm,
            fill,
            stroke,
            opa: st.group_opa,
            fill_opa: st.fill_opa.mul(fill_extra),
            stroke_opa: st.stroke_opa.mul(stroke_extra),
            ..VectorDsc::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_lists() {
        let t = parse_transform("translate(10) scale(2)").unwrap();
        // Scale first, then translate.
        assert_eq!(t.map(Fx::ONE, Fx::ONE), (Fx::from_int(12), Fx::from_int(2)));
        let r = parse_transform("rotate(90 10 10)").unwrap();
        assert_eq!(
            r.map_point(twine_core::Point::new(20, 10)),
            twine_core::Point::new(10, 20)
        );
        assert!(parse_transform("scale(1,2,3)").is_none());
        assert!(parse_transform("foo(1)").is_none());
        assert!(parse_transform("translate(1").is_none());
        assert_eq!(parse_transform(" ").unwrap(), Transform::IDENTITY);
        let m = parse_transform("matrix(1 0 0 1 5 6),skewX(0)").unwrap();
        assert_eq!(m, Transform::translate(Fx::from_int(5), Fx::from_int(6)));
    }

    #[test]
    fn opacity_values() {
        assert_eq!(parse_opacity("0.5"), Some(Opa(128)));
        assert_eq!(parse_opacity("50%"), Some(Opa(128)));
        assert_eq!(parse_opacity("2"), Some(Opa(255)));
        assert_eq!(parse_opacity("-1"), Some(Opa(0)));
        assert_eq!(parse_opacity("x"), None);
    }

    #[test]
    fn view_box_fit() {
        let doc = parse_svg(br#"<svg viewBox="0 0 20 10"/>"#).unwrap();
        let t = doc.transform_for(Rect::new(0, 0, 100, 100));
        // meet: scale 5, centered vertically (free 50 → 25).
        assert_eq!(
            t.map_point(twine_core::Point::new(0, 0)),
            twine_core::Point::new(0, 25)
        );
        assert_eq!(
            t.map_point(twine_core::Point::new(20, 10)),
            twine_core::Point::new(100, 75)
        );
        let doc = parse_svg(br#"<svg viewBox="0 0 20 10" preserveAspectRatio="xMaxYMax slice"/>"#).unwrap();
        let t = doc.transform_for(Rect::new(0, 0, 100, 100));
        assert_eq!(
            t.map_point(twine_core::Point::new(20, 10)),
            twine_core::Point::new(100, 100)
        );
        let doc = parse_svg(br#"<svg width="30" height="40" preserveAspectRatio="none"/>"#).unwrap();
        assert_eq!(doc.size, FxSize::new(Fx::from_int(30), Fx::from_int(40)));
        let t = doc.transform_for(Rect::new(10, 10, 70, 50));
        assert_eq!(
            t.map_point(twine_core::Point::new(30, 40)),
            twine_core::Point::new(70, 50)
        );
    }
}
