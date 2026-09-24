//! The typed Lottie (Bodymovin JSON) model produced by [`load`](crate::load).
//!
//! Field names follow the JSON keys where they are well known (`w`, `h`, `fr`,
//! `ip`, `op`, `ks`, …) so the model can be read side by side with a Lottie file. Only the
//! supported subset is represented; everything else is recorded in
//! [`Composition::unsupported`] and ignored.
//!
//! Colors are `0..=1` floats, opacities and percentages `0..=100`, angles degrees, times frames.

use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;

/// A 2-D value (point, size, scale in percent).
pub type Vec2 = [f32; 2];

/// An RGBA color with `0..=1` channels (Lottie ignores the alpha channel of shape colors).
pub type Rgba = [f32; 4];

/// A whole animation: the root layers, the precomposition assets and the timing.
#[derive(Clone, Debug, PartialEq)]
pub struct Composition {
    /// Width in composition units.
    pub w: f32,
    /// Height in composition units.
    pub h: f32,
    /// Frame rate (frames per second, > 0).
    pub fr: f32,
    /// First frame.
    pub ip: f32,
    /// Frame after the last one (`op > ip`).
    pub op: f32,
    /// Root layers, top-most first (as in the file).
    pub layers: Vec<Layer>,
    /// Precomposition assets referenced by precomp layers ([`Layer::asset`]).
    pub assets: Vec<Asset>,
    /// Unsupported features found in the file (sorted, each listed once).
    pub unsupported: Vec<String>,
}

impl Composition {
    /// Number of frames (`op − ip`).
    #[must_use]
    pub fn frame_count(&self) -> f32 {
        self.op - self.ip
    }

    /// Duration in seconds.
    #[must_use]
    pub fn duration_secs(&self) -> f32 {
        self.frame_count() / self.fr
    }

    /// Unsupported features found in the file (each listed once).
    #[must_use]
    pub fn unsupported(&self) -> &[String] {
        &self.unsupported
    }
}

/// A precomposition: a list of layers used by precomp layers.
#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    /// Asset id (`id`), referenced by [`Layer::ref_id`].
    pub id: String,
    /// The layers, top-most first.
    pub layers: Vec<Layer>,
}

/// Layer type (`ty`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum LayerKind {
    /// `0`: renders a precomposition asset.
    Precomp,
    /// `1`: a solid color rectangle.
    Solid,
    /// `3`: invisible, used as a parent.
    Null,
    /// `4`: vector shapes.
    Shape,
    /// Any other type (image, text, audio, …): not rendered.
    Unsupported(u8),
}

/// One layer.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// Layer type.
    pub ty: LayerKind,
    /// Name (`nm`), for logs.
    pub name: String,
    /// Index used by `parent` and `tp` references.
    pub ind: Option<i32>,
    /// `ind` of the parent layer.
    pub parent: Option<i32>,
    /// Resolved index of the parent in the same layer list.
    pub parent_index: Option<usize>,
    /// First visible frame (in the containing composition's time).
    pub ip: f32,
    /// Frame after the last visible one.
    pub op: f32,
    /// Start time offset: the layer's local time is `(frame − st) / sr`.
    pub st: f32,
    /// Time stretch (`sr`, 1 = normal).
    pub sr: f32,
    /// Layer transform.
    pub ks: Transform,
    /// Shapes (shape layers), top-most first.
    pub shapes: Vec<Shape>,
    /// Masks, applied in order.
    pub masks: Vec<Mask>,
    /// Track matte type of this layer (`1` alpha, `2` inverted alpha; `3`/`4` luma are
    /// unsupported).
    pub tt: Option<u8>,
    /// `1` if this layer is the track matte of the next layer (not rendered by itself).
    pub td: Option<u8>,
    /// `ind` of the matte layer (newer files); `None` = the layer above.
    pub tp: Option<i32>,
    /// Resolved index of the matte source layer in the same list.
    pub matte_index: Option<usize>,
    /// Precomp asset id (`refId`).
    pub ref_id: Option<String>,
    /// Resolved index of the asset in [`Composition::assets`].
    pub asset: Option<usize>,
    /// Precomp width (clip).
    pub w: f32,
    /// Precomp height (clip).
    pub h: f32,
    /// Time remapping of a precomp in seconds (`tm`).
    pub tm: Option<Animatable<f32>>,
    /// Solid color (`sc`, RGB `0..=1`).
    pub sc: Rgba,
    /// Solid width.
    pub sw: f32,
    /// Solid height.
    pub sh: f32,
    /// Hidden layers are not rendered (`hd`).
    pub hidden: bool,
}

/// A transform (layer `ks`, group `tr`, repeater `tr`).
///
/// Points are mapped as `translate(p) · rotate(r) · skew(sk, sa) · scale(s / 100) ·
/// translate(−a)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Transform {
    /// Anchor point `a`.
    pub anchor: Animatable<Vec2>,
    /// Position `p` (or split `px`/`py`).
    pub position: Position,
    /// Scale `s` in percent.
    pub scale: Animatable<Vec2>,
    /// Rotation `r` (or `rz`) in degrees, clockwise.
    pub rotation: Animatable<f32>,
    /// Opacity `o` in percent.
    pub opacity: Animatable<f32>,
    /// Skew `sk` in degrees.
    pub skew: Animatable<f32>,
    /// Skew axis `sa` in degrees.
    pub skew_axis: Animatable<f32>,
    /// Repeater start opacity `so` (percent; repeater transforms only).
    pub start_opacity: Animatable<f32>,
    /// Repeater end opacity `eo` (percent; repeater transforms only).
    pub end_opacity: Animatable<f32>,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            anchor: Animatable::Static([0.0, 0.0]),
            position: Position::Combined(Animatable::Static([0.0, 0.0])),
            scale: Animatable::Static([100.0, 100.0]),
            rotation: Animatable::Static(0.0),
            opacity: Animatable::Static(100.0),
            skew: Animatable::Static(0.0),
            skew_axis: Animatable::Static(0.0),
            start_opacity: Animatable::Static(100.0),
            end_opacity: Animatable::Static(100.0),
        }
    }
}

/// A position: one 2-D property or separate x and y properties.
#[derive(Clone, Debug, PartialEq)]
pub enum Position {
    /// `p` as one (possibly spatially interpolated) value.
    Combined(Animatable<Vec2>),
    /// `p.s == true`: separate `x` and `y`.
    Split(Animatable<f32>, Animatable<f32>),
}

/// A property that is either constant or animated by keyframes.
#[derive(Clone, Debug, PartialEq)]
pub enum Animatable<T> {
    /// A constant value.
    Static(T),
    /// Keyframes (at least two).
    Keyframed(Keyframes<T>),
}

impl<T> Animatable<T> {
    /// Whether the value changes over time.
    #[must_use]
    pub fn is_animated(&self) -> bool {
        matches!(self, Animatable::Keyframed(_))
    }
}

/// Keyframes of an animated property, sorted by time, plus the index of the segment used last
/// (so sequential playback finds the segment in O(1)).
#[derive(Clone, Debug, PartialEq)]
pub struct Keyframes<T> {
    /// The keyframes (at least two).
    pub frames: Vec<Keyframe<T>>,
    /// Segment used by the last evaluation.
    pub(crate) last: Cell<u32>,
}

impl<T> Keyframes<T> {
    /// Keyframes from a list sorted by time.
    #[must_use]
    pub fn new(frames: Vec<Keyframe<T>>) -> Self {
        Self {
            frames,
            last: Cell::new(0),
        }
    }
}

/// One keyframe: the value at time `t` and how it moves on to the next keyframe.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe<T> {
    /// Time (frame, in the layer's local time).
    pub t: f32,
    /// Start value `s`.
    pub s: T,
    /// End value `e` of older files; `None` = the next keyframe's `s`.
    pub e: Option<T>,
    /// Hold keyframe (`h: 1`): the value jumps at the next keyframe.
    pub hold: bool,
    /// Easing towards the next keyframe (`o`/`i`); `None` = linear.
    pub ease: Option<Ease>,
    /// Spatial out tangent `to` (positions), relative to `s`.
    pub to: Option<Vec2>,
    /// Spatial in tangent `ti` (positions), relative to the end value.
    pub ti: Option<Vec2>,
}

/// Cubic-bezier easing of one keyframe segment, per axis (Lottie allows a different curve per
/// dimension). `x` is time progress, `y` value progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ease {
    /// Out control point `(x1, y1)` per axis (`o`).
    pub out: [Vec2; 4],
    /// In control point `(x2, y2)` per axis (`i`).
    pub inn: [Vec2; 4],
    /// Number of axes given (1..=4); later axes reuse the last one.
    pub axes: u8,
}

/// A bezier path: vertices with in/out tangents (relative to their vertex).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PathData {
    /// Closed path (`c`).
    pub closed: bool,
    /// Vertices `v`.
    pub v: Vec<Vec2>,
    /// In tangents `i`.
    pub i: Vec<Vec2>,
    /// Out tangents `o`.
    pub o: Vec<Vec2>,
}

/// A shape item (`shapes` / group `it`).
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// `gr`: a group with its own transform.
    Group(Group),
    /// `rc`: rectangle.
    Rect(Rectangle),
    /// `el`: ellipse.
    Ellipse(Ellipse),
    /// `sh`: bezier path.
    Path(PathShape),
    /// `fl`: solid fill.
    Fill(Fill),
    /// `st`: solid stroke.
    Stroke(Stroke),
    /// `gf`: gradient fill.
    GradientFill(GradientFill),
    /// `gs`: gradient stroke.
    GradientStroke(GradientStroke),
    /// `tm`: trim paths.
    Trim(TrimPaths),
    /// `rp`: repeater.
    Repeater(Repeater),
    /// Any other item type (ignored; name recorded in [`Composition::unsupported`]).
    Unsupported(String),
}

impl Shape {
    /// Number of items in this item's subtree, itself included (group transforms are not
    /// items). Used to number items in depth-first order.
    #[must_use]
    pub fn size(&self) -> u32 {
        match self {
            Shape::Group(g) => g.size,
            _ => 1,
        }
    }
}

/// A shape group.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// The items (without the transform), top-most first.
    pub items: Vec<Shape>,
    /// The group transform (`tr` item).
    pub transform: Transform,
    /// 1 + the sizes of all items.
    pub size: u32,
}

/// `rc`: a rectangle centred on `p`.
#[derive(Clone, Debug, PartialEq)]
pub struct Rectangle {
    /// Center.
    pub p: Animatable<Vec2>,
    /// Size.
    pub s: Animatable<Vec2>,
    /// Corner roundness.
    pub r: Animatable<f32>,
    /// Counter-clockwise direction (`d: 3`).
    pub reversed: bool,
}

/// `el`: an ellipse centred on `p`.
#[derive(Clone, Debug, PartialEq)]
pub struct Ellipse {
    /// Center.
    pub p: Animatable<Vec2>,
    /// Size (diameters).
    pub s: Animatable<Vec2>,
    /// Counter-clockwise direction (`d: 3`).
    pub reversed: bool,
}

/// `sh`: a bezier path.
#[derive(Clone, Debug, PartialEq)]
pub struct PathShape {
    /// The path.
    pub ks: Animatable<PathData>,
    /// Reversed direction (`d: 3`).
    pub reversed: bool,
}

/// Fill rule (`r`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum FillRule {
    /// `1`: non-zero winding.
    #[default]
    NonZero,
    /// `2`: even-odd.
    EvenOdd,
}

/// `fl`: solid fill of the preceding geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct Fill {
    /// Color `c`.
    pub c: Animatable<Rgba>,
    /// Opacity `o` (percent).
    pub o: Animatable<f32>,
    /// Fill rule `r`.
    pub rule: FillRule,
}

/// Line cap (`lc`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum LineCap {
    /// `1`.
    Butt,
    /// `2`.
    #[default]
    Round,
    /// `3`.
    Square,
}

/// Line join (`lj`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum LineJoin {
    /// `1`.
    Miter,
    /// `2`.
    #[default]
    Round,
    /// `3`.
    Bevel,
}

/// Kind of a dash entry (`n`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DashKind {
    /// `d`: dash length.
    Dash,
    /// `g`: gap length.
    Gap,
    /// `o`: offset of the pattern.
    Offset,
}

/// One dash entry of a stroke (`d` array).
#[derive(Clone, Debug, PartialEq)]
pub struct DashEntry {
    /// What the value means.
    pub kind: DashKind,
    /// Length (stroke units).
    pub v: Animatable<f32>,
}

/// Stroke parameters shared by solid and gradient strokes.
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeStyle {
    /// Width `w`.
    pub w: Animatable<f32>,
    /// Line cap.
    pub lc: LineCap,
    /// Line join.
    pub lj: LineJoin,
    /// Miter limit `ml`.
    pub ml: f32,
    /// Dash pattern.
    pub dash: Vec<DashEntry>,
}

/// `st`: solid stroke of the preceding geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    /// Color `c`.
    pub c: Animatable<Rgba>,
    /// Opacity `o` (percent).
    pub o: Animatable<f32>,
    /// Width, caps, joins and dashes.
    pub style: StrokeStyle,
}

/// Gradient type (`t`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum GradientKind {
    /// `1`.
    Linear,
    /// `2`.
    Radial,
}

/// Gradient geometry and colors shared by `gf` and `gs`.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    /// Linear or radial.
    pub kind: GradientKind,
    /// Start point `s` (radial: center).
    pub s: Animatable<Vec2>,
    /// End point `e` (radial: a point on the end circle).
    pub e: Animatable<Vec2>,
    /// Highlight length `h` (percent of the radius, radial only).
    pub h: Animatable<f32>,
    /// Highlight angle `a` (degrees, radial only).
    pub a: Animatable<f32>,
    /// Number of color stops `g.p`.
    pub points: u32,
    /// Flat stop data `g.k`: `points × [offset, r, g, b]`, then optional `[offset, alpha]`
    /// pairs.
    pub stops: Animatable<Vec<f32>>,
}

/// `gf`: gradient fill.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientFill {
    /// The gradient.
    pub g: Gradient,
    /// Opacity `o` (percent).
    pub o: Animatable<f32>,
    /// Fill rule.
    pub rule: FillRule,
}

/// `gs`: gradient stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientStroke {
    /// The gradient.
    pub g: Gradient,
    /// Opacity `o` (percent).
    pub o: Animatable<f32>,
    /// Width, caps, joins and dashes.
    pub style: StrokeStyle,
}

/// Trim mode (`m`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TrimMode {
    /// `1`: every path is trimmed by the same range.
    #[default]
    Simultaneously,
    /// `2`: the paths are trimmed as one concatenated path.
    Individually,
}

/// `tm`: trim paths of the preceding geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct TrimPaths {
    /// Start `s` (percent).
    pub s: Animatable<f32>,
    /// End `e` (percent).
    pub e: Animatable<f32>,
    /// Offset `o` (degrees; 360 = one full path length).
    pub o: Animatable<f32>,
    /// Mode.
    pub m: TrimMode,
}

/// Repeater composite order (`m`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Composite {
    /// `1`: later copies above earlier ones.
    #[default]
    Above,
    /// `2`: later copies below earlier ones.
    Below,
}

/// `rp`: repeats the preceding items of the group.
#[derive(Clone, Debug, PartialEq)]
pub struct Repeater {
    /// Number of copies `c`.
    pub c: Animatable<f32>,
    /// Offset `o` (copies).
    pub o: Animatable<f32>,
    /// Stacking order.
    pub m: Composite,
    /// Transform applied `k` times to copy `k` (with `so`/`eo` opacities).
    pub tr: Transform,
}

/// Mask mode (`mode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MaskMode {
    /// `a`: union.
    Add,
    /// `s`: subtract.
    Subtract,
    /// `i`: intersect.
    Intersect,
    /// `n` (or an unsupported mode): ignored.
    None,
}

/// One layer mask (`masksProperties`).
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    /// How the mask combines with the previous ones.
    pub mode: MaskMode,
    /// Mask path `pt` (layer space).
    pub pt: Animatable<PathData>,
    /// Opacity `o` (percent).
    pub o: Animatable<f32>,
    /// Inverted (`inv`).
    pub inv: bool,
}
