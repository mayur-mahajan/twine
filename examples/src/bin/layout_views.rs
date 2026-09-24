//! `cargo xtask sim layout_views`: the layout containers — a column with rows and a grid of
//! colored boxes. The button toggles the gap through a signal: one binding runs, the layout
//! of the affected containers is redone and only what moved is redrawn (F2).

use twine::prelude::*;
use twine_sim::SimConfig;

static COLS: [GridTrack; 4] = [
    GridTrack::Fr(1),
    GridTrack::Fr(1),
    GridTrack::Fr(1),
    GridTrack::Fr(1),
];
static ROWS: [GridTrack; 2] = [GridTrack::Px(36), GridTrack::Px(36)];

const COLORS: [u32; 8] = [
    0xE5_39_35, 0x8E_24_AA, 0x39_49_AB, 0x03_9B_E5, 0x00_89_7B, 0x7C_B3_42, 0xFD_D8_35, 0xFB_8C_00,
];

fn tile(c: u32) -> Container {
    container(()).bg(Color::hex(c)).border_width(0).radius(4)
}

fn app(cx: Scope) -> impl View {
    let wide = cx.signal(false);
    let gap = move || if wide.get() { 12 } else { 4 };
    let cells: Vec<_> = COLORS
        .iter()
        .enumerate()
        .map(|(i, c)| {
            tile(*c)
                .grid_cell((i % 4) as i32, 1, (i / 4) as i32, 1)
                .grid_cell_align(GridAlign::Stretch, GridAlign::Stretch)
        })
        .collect();
    column((
        row((
            button(label("Toggle gap")).on_click(move || wide.update(|w| *w = !*w)),
            label(text!("gap = {}", gap())),
        ))
        .gap(10)
        .align_items(FlexAlign::Center),
        row((
            tile(COLORS[0]).size(40, 30),
            tile(COLORS[1]).size(60, 30),
            tile(COLORS[2]).size(30, 30),
        ))
        .gap(gap),
        grid(&COLS, &ROWS, cells).gap(gap).width(Length::pct(100)),
    ))
    .gap(gap)
    .padding(8)
    .size(Length::pct(100), Length::pct(100))
}

fn main() {
    twine_sim::run(SimConfig::new(320, 240).title("layout views").scale(2), app);
}
