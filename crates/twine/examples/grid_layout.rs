//! `cargo xtask sim grid_layout`: an interactive grid layout playground.
//!
//! A 3 × 3 grid (mixed `Px`, `Content` and `Fr` tracks) with eight labelled children; the
//! selected child has a dark outline. The window title shows the current configuration (also
//! logged). Keys:
//!
//! - `c` / `r`: cycle the column / row track alignment (start, center, end, stretch, space
//!   evenly / around / between).
//! - Tab: select the next child.
//! - `x` / `y`: cycle the horizontal / vertical cell alignment of the selected child.
//! - `h` / `v`: cycle the column / row span of the selected child (1–3).
//! - `t`: switch between track templates (`px / content / fr`, all `fr`, all `content`).
//!
//! F2 shows the refreshed areas, F3 the layout bounds.

mod common;

use common::tile::{PALETTE, tile};
use twine::assets::fonts::MONTSERRAT_14;
use twine::core::{Color, Opa};
use twine::engine::{Engine, NodeId};
use twine::hal::Key;
use twine::style::{Align, GridAlign, GridSpan, GridTrack, LayoutKind, Length, Radius, Selector, StyleProp};
use twine_sim::{SimConfig, run_engine};

const ALIGNS: [GridAlign; 7] = [
    GridAlign::Start,
    GridAlign::Center,
    GridAlign::End,
    GridAlign::Stretch,
    GridAlign::SpaceEvenly,
    GridAlign::SpaceAround,
    GridAlign::SpaceBetween,
];

/// Cell alignments (the space modes only apply to tracks).
const CELL_ALIGNS: [GridAlign; 4] = [
    GridAlign::Start,
    GridAlign::Center,
    GridAlign::End,
    GridAlign::Stretch,
];

/// `(name, columns, rows)` templates.
static TEMPLATES: [(&str, &[GridTrack], &[GridTrack]); 3] = [
    (
        "px/content/fr",
        &[GridTrack::Px(60), GridTrack::Content, GridTrack::Fr(1)],
        &[GridTrack::Content, GridTrack::Px(50), GridTrack::Fr(1)],
    ),
    (
        "fr 1/2/1",
        &[GridTrack::Fr(1), GridTrack::Fr(2), GridTrack::Fr(1)],
        &[GridTrack::Fr(1), GridTrack::Fr(1), GridTrack::Fr(2)],
    ),
    (
        "content",
        &[GridTrack::Content, GridTrack::Content, GridTrack::Content],
        &[GridTrack::Content, GridTrack::Content, GridTrack::Content],
    ),
];

/// Labels of the children (their content sizes differ).
const LABELS: [&str; 8] = ["1", "two", "3", "four 4", "5", "six", "seven 7", "8"];

/// The playground's configuration.
#[derive(Debug, Clone, Copy, Default)]
struct Config {
    col_align: usize,
    row_align: usize,
    template: usize,
    selected: usize,
}

/// Per-child cell settings (indices into [`CELL_ALIGNS`] and spans).
#[derive(Debug, Clone, Copy)]
struct Cell {
    x_align: usize,
    y_align: usize,
    col_span: i32,
    row_span: i32,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            x_align: 1,
            y_align: 1,
            col_span: 1,
            row_span: 1,
        }
    }
}

struct Playground {
    grid: NodeId,
    cfg: Config,
    cells: [Cell; 8],
}

impl Playground {
    fn child(&self, e: &Engine, i: usize) -> Option<NodeId> {
        e.tree().child(self.grid, i as i32)
    }

    fn describe(&self) -> String {
        let c = self.cells[self.cfg.selected];
        format!(
            "grid_layout — {} cols={:?} rows={:?} | child {}: x={:?} y={:?} span {}×{}",
            TEMPLATES[self.cfg.template].0,
            ALIGNS[self.cfg.col_align],
            ALIGNS[self.cfg.row_align],
            LABELS[self.cfg.selected],
            CELL_ALIGNS[c.x_align],
            CELL_ALIGNS[c.y_align],
            c.col_span,
            c.row_span
        )
    }

    /// Applies everything (idempotent setters: only changes mark the layout).
    fn apply(&self, e: &mut Engine) {
        let (_, cols, rows) = TEMPLATES[self.cfg.template];
        e.set_grid_tracks(self.grid, cols, rows);
        e.set_grid_align(self.grid, ALIGNS[self.cfg.col_align], ALIGNS[self.cfg.row_align]);
        for (i, c) in self.cells.iter().enumerate() {
            let Some(n) = self.child(e, i) else { continue };
            let (col, row) = ((i % 3) as i32, (i / 3) as i32);
            e.set_grid_cell(
                n,
                GridSpan::new(col, c.col_span),
                GridSpan::new(row, c.row_span),
                CELL_ALIGNS[c.x_align],
                CELL_ALIGNS[c.y_align],
            );
            let w = if i == self.cfg.selected { 2 } else { 0 };
            e.set_local_prop(n, Selector::MAIN, StyleProp::OutlineWidth(w));
        }
        twine_sim::set_title(&self.describe());
    }

    /// Handles the example's keys; returns whether something changed.
    fn on_key(&mut self, e: &mut Engine, key: Key) -> bool {
        let sel = self.cfg.selected;
        match key {
            Key::Next => self.cfg.selected = (sel + 1) % self.cells.len(),
            Key::Prev => self.cfg.selected = (sel + self.cells.len() - 1) % self.cells.len(),
            Key::Char('c') => self.cfg.col_align = (self.cfg.col_align + 1) % ALIGNS.len(),
            Key::Char('r') => self.cfg.row_align = (self.cfg.row_align + 1) % ALIGNS.len(),
            Key::Char('t') => self.cfg.template = (self.cfg.template + 1) % TEMPLATES.len(),
            Key::Char('x') => self.cells[sel].x_align = (self.cells[sel].x_align + 1) % CELL_ALIGNS.len(),
            Key::Char('y') => self.cells[sel].y_align = (self.cells[sel].y_align + 1) % CELL_ALIGNS.len(),
            Key::Char('h') => self.cells[sel].col_span = self.cells[sel].col_span % 3 + 1,
            Key::Char('v') => self.cells[sel].row_span = self.cells[sel].row_span % 3 + 1,
            _ => return false,
        }
        self.apply(e);
        true
    }
}

fn scene(e: &mut Engine) -> Playground {
    e.config_mut().default_font = Some(&MONTSERRAT_14);
    let d = e.default_display().expect("display");
    let screen = e.active_screen(d).expect("screen");
    e.set_local_prop(
        screen,
        Selector::MAIN,
        StyleProp::BgColor(Color::hex(0xEC_EF_F1).into()),
    );
    e.set_local_prop(screen, Selector::MAIN, StyleProp::BgOpacity(Opa::COVER.into()));
    let grid = e.create(screen, Box::new(twine::engine::Obj)).expect("grid");
    e.set_size(grid, Length::pct(94), Length::pct(92));
    e.align(grid, Align::Center, 0, 0);
    e.set_layout(grid, LayoutKind::Grid);
    for p in [
        StyleProp::BgColor(Color::WHITE.into()),
        StyleProp::BgOpacity(Opa::COVER.into()),
        StyleProp::Radius(Radius::Px(8).into()),
        StyleProp::BorderWidth(Length::Px(1).into()),
        StyleProp::BorderColor(Color::hex(0xB0_BE_C5).into()),
        StyleProp::PaddingLeft(Length::Px(8).into()),
        StyleProp::PaddingRight(Length::Px(8).into()),
        StyleProp::PaddingTop(Length::Px(8).into()),
        StyleProp::PaddingBottom(Length::Px(8).into()),
        StyleProp::RowGap(Length::Px(6).into()),
        StyleProp::ColumnGap(Length::Px(6).into()),
    ] {
        e.set_local_prop(grid, Selector::MAIN, p);
    }
    for (i, label) in LABELS.iter().enumerate() {
        let t = tile(e, grid, *label, Color::hex(PALETTE[i % PALETTE.len()]));
        e.set_local_prop(
            t,
            Selector::MAIN,
            StyleProp::OutlineColor(Color::hex(0x21_21_21).into()),
        );
        e.set_local_prop(t, Selector::MAIN, StyleProp::OutlineOffset(1));
        // Content-sized with a minimum, so `Content` tracks follow the labels.
        e.set_local_prop(t, Selector::MAIN, StyleProp::MinWidth(Length::Px(24).into()));
        e.set_local_prop(t, Selector::MAIN, StyleProp::MinHeight(Length::Px(24).into()));
    }
    let p = Playground {
        grid,
        cfg: Config::default(),
        cells: [Cell::default(); 8],
    };
    p.apply(e);
    p
}

fn main() {
    twine_sim::init_logging();
    let pg: std::rc::Rc<std::cell::RefCell<Option<Playground>>> = std::rc::Rc::default();
    let pg2 = pg.clone();
    let sim = SimConfig::new(320, 240)
        .title("grid_layout")
        .scale(2)
        .on_raw_key(move |e, k| {
            if let Some(p) = pg2.borrow_mut().as_mut() {
                p.on_key(e, k);
            }
        });
    run_engine(sim, move |e| *pg.borrow_mut() = Some(scene(e)));
}
