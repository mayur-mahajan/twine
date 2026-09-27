//! [`Tileview`]: a 2D grid of full-size tiles navigated by swipes in the allowed directions
//! (LVGL `lv_tileview`).

use alloc::boxed::Box;

use twine_engine::{
    Engine, EngineError, Event, EventCode, EventCx, EventParam, EventResult, NodeId, OBJ_FLAGS, ObjFlags,
    Widget, WidgetClass, WidgetCx, fmt_node_id,
};
use twine_style::{Dir, Length, ScrollSnap};

use crate::util::log_set;

/// The class of [`Tileview`]: `"tileview"`, the base object's parts and flags.
pub static TILEVIEW_CLASS: WidgetClass = WidgetClass::new("tileview")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS);
/// The class of [`Tile`]: `"tileview_tile"`.
pub static TILEVIEW_TILE_CLASS: WidgetClass = WidgetClass::new("tileview_tile")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS);

/// A tile (LVGL `lv_tileview_tile`): a container as large as the tileview, at column `col`
/// and row `row`, with the directions `dir` the user may swipe to from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Tile {
    col: u8,
    row: u8,
    dir: Dir,
}

impl Tile {
    /// A tile at `(col, row)` leaving in `dir`.
    #[must_use]
    pub const fn new(col: u8, row: u8, dir: Dir) -> Self {
        Self { col, row, dir }
    }

    /// The column.
    #[must_use]
    pub fn col(&self) -> u8 {
        self.col
    }

    /// The row.
    #[must_use]
    pub fn row(&self) -> u8 {
        self.row
    }

    /// The directions the user may swipe to from this tile.
    #[must_use]
    pub fn dir(&self) -> Dir {
        self.dir
    }
}

impl Widget for Tile {
    fn class(&self) -> &'static WidgetClass {
        &TILEVIEW_TILE_CLASS
    }

    /// LVGL `lv_tileview_tile_constructor` + `lv_tileview_add_tile`: 100 % × 100 % at
    /// `(col × 100 %, row × 100 %)`.
    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let id = cx.node();
        let e = cx.engine_mut();
        e.set_size(id, Length::pct(100), Length::pct(100));
        e.set_pos(
            id,
            Length::pct(i16::from(self.col) * 100),
            Length::pct(i16::from(self.row) * 100),
        );
    }
}

/// A tileview (LVGL `lv_tileview`): tiles fill the view and sit on a grid; the view scrolls
/// one tile at a time, snapping to tile centers, only in the directions the active tile
/// allows (LVGL changes the view's `scroll_dir` after each scroll).
///
/// - **Events**: `ValueChanged` on the tileview when a scroll ends on another tile; the tile
///   is [`tile_active`](Self::tile_active).
/// - Tiles follow the view's size (percentages), so a resize keeps them aligned.
///
/// ```
/// use twine_style::Dir;
/// use twine_testing::EngineHarness;
/// use twine_widgets_ext::tileview::{self, Tileview};
///
/// let mut h = EngineHarness::new(240, 240);
/// let screen = h.screen();
/// let tv = tileview::create(h.engine_mut(), screen).unwrap();
/// let t1 = tileview::add_tile(h.engine_mut(), tv, 0, 0, Dir::RIGHT).unwrap();
/// let t2 = tileview::add_tile(h.engine_mut(), tv, 1, 0, Dir::LEFT).unwrap();
/// h.run_until_idle();
/// h.engine_mut().with_widget_mut(tv, |w: &mut Tileview, cx| w.set_tile(cx, t2, false));
/// assert_eq!(h.engine().widget::<Tileview>(tv).unwrap().tile_active(), Some(t2));
/// # let _ = t1;
/// ```
#[derive(Debug, Default)]
pub struct Tileview {
    tile_act: Option<NodeId>,
}

impl Tileview {
    /// A tileview.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The tile in view (LVGL `lv_tileview_get_tile_active`).
    #[must_use]
    pub fn tile_active(&self) -> Option<NodeId> {
        self.tile_act
    }

    /// Scrolls to `tile` (animated when `anim`) and allows the swipes of that tile (LVGL
    /// `lv_tileview_set_tile`). No `ValueChanged` until the scroll ends.
    pub fn set_tile(&mut self, cx: &mut WidgetCx<'_>, tile: NodeId, anim: bool) {
        let tv = cx.node();
        let e = cx.engine_mut();
        let Some(t) = e.widget::<Tile>(tile).copied() else {
            twine_core::warn!(target: "twine::engine", "tileview: {} is not a tile", fmt_node_id(tile));
            return;
        };
        log_set(TILEVIEW_CLASS.name, tv, "tile");
        e.update_layout();
        let (tx, ty) = tile_origin(e, tv, t);
        self.tile_act = Some(tile);
        e.set_scroll_dir(tv, t.dir);
        e.scroll_to(tv, tx, ty, anim);
    }

    /// Scrolls to the tile at `(col, row)` (LVGL `lv_tileview_set_tile_by_index`). A missing
    /// tile logs `warn!`.
    pub fn set_tile_by_index(&mut self, cx: &mut WidgetCx<'_>, col: u8, row: u8, anim: bool) {
        let tv = cx.node();
        let found = cx.engine().tree().children(tv).find(|&c| {
            cx.engine()
                .widget::<Tile>(c)
                .is_some_and(|t| t.col == col && t.row == row)
        });
        match found {
            Some(t) => self.set_tile(cx, t, anim),
            None => twine_core::warn!(target: "twine::engine", "tileview: no tile at ({}, {})", col, row),
        }
    }

    /// LVGL `tileview_event_cb` (`SCROLL_END`): the tile in view becomes active, the scroll
    /// directions become its own.
    fn scroll_end(&mut self, cx: &mut WidgetCx<'_>) {
        let tv = cx.node();
        let e = cx.engine_mut();
        if e.active_input().is_some_and(|i| e.input_pressed(i)) {
            return;
        }
        let ca = e.content_area(tv);
        let (w, h) = (ca.width().max(1), ca.height().max(1));
        let p = e.scroll_end(tv);
        let tx = ((p.x + w / 2) / w) * w;
        let ty = ((p.y + h / 2) / h) * h;
        let mut dir = Dir::ALL;
        let hit = e.tree().children(tv).find_map(|c| {
            let t = e.widget::<Tile>(c)?;
            (tile_origin(e, tv, *t) == (tx, ty)).then_some((c, t.dir))
        });
        if let Some((tile, d)) = hit {
            dir = d;
            let changed = self.tile_act != Some(tile);
            self.tile_act = Some(tile);
            if changed {
                twine_core::debug!(target: "twine::engine", "tileview#{} tile {}", fmt_node_id(tv), fmt_node_id(tile));
                cx.post_event(EventCode::ValueChanged, EventParam::None);
            }
        }
        cx.engine_mut().set_scroll_dir(tv, dir);
    }
}

/// The scroll offset of tile `t`: its column and row times the view's content size.
fn tile_origin(e: &Engine, tv: NodeId, t: Tile) -> (i32, i32) {
    let ca = e.content_area(tv);
    (i32::from(t.col) * ca.width(), i32::from(t.row) * ca.height())
}

/// Creates a tileview, 100 % × 100 % of its parent, as the last child of `parent` (LVGL
/// `lv_tileview_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Tileview::new()))
}

/// Adds a tile at `(col, row)` that may be left in `dir` (LVGL `lv_tileview_add_tile`). The
/// tile at `(0, 0)` sets the view's first scroll directions.
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `tv` does not exist (logged).
pub fn add_tile(e: &mut Engine, tv: NodeId, col: u8, row: u8, dir: Dir) -> Result<NodeId, EngineError> {
    let tile = e.create(tv, Box::new(Tile::new(col, row, dir)))?;
    if col == 0 && row == 0 {
        e.set_scroll_dir(tv, dir);
        e.with_widget_mut(tv, |w: &mut Tileview, _| {
            if w.tile_act.is_none() {
                w.tile_act = Some(tile);
            }
        });
    }
    Ok(tile)
}

impl Widget for Tileview {
    fn class(&self) -> &'static WidgetClass {
        &TILEVIEW_CLASS
    }

    /// LVGL `lv_tileview_constructor`: 100 % × 100 %, one tile per scroll, snapping to tile
    /// centers on both axes.
    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let id = cx.node();
        let e = cx.engine_mut();
        e.set_size(id, Length::pct(100), Length::pct(100));
        e.set_flag(id, ObjFlags::SCROLL_ONE, true);
        e.set_scroll_snap_x(id, ScrollSnap::Center);
        e.set_scroll_snap_y(id, ScrollSnap::Center);
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.target == cx.node() && ev.code == EventCode::ScrollEnd {
            self.scroll_end(&mut cx.widget_cx());
        }
        EventResult::Continue
    }
}
