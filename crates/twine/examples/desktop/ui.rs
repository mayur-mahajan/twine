//! The app's building blocks: the [`App`] store every screen receives, keyboard focus, the
//! text styles, flat icon buttons, and a small set of line icons drawn on a canvas.
//!
//! **Focus and shortcuts.** Twine sends keys to the focused node of the keypad's focus
//! group. The app keeps exactly two nodes in that group: the root of the window, which owns
//! the shortcuts (Space, arrows, F), and the search field. Every other control is
//! deliberately *not* focusable, so clicking a button never steals the shortcuts; and every
//! button gives the focus back to the root when pressed ([`Focus::release`]), so a click
//! outside the search field ends typing — the behaviour people expect from a desktop app.
//!
//! **Icons.** The bundled symbol font has transport glyphs but no library icons (an album, an
//! artist, a search glass), so those are drawn: [`icon`] is a canvas painting a few lines and
//! circles on a 20-unit grid scaled to the node, in the accent or the muted color.

use twine::prelude::*;
use twine::render::ArcDsc;
use twine::widgets::slider::Slider;

use crate::canvas::{Canvas, canvas};
use crate::eq::Equalizer;
use crate::paint::{circle, line, ring};
use crate::player::Player;
use crate::theme::{self, BODY, CAPTION, FAINT, OVERLINE, PRESSED, RAISED, palette};
use crate::viz::VizMode;

/// The screens of the main area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Songs,
    Albums,
    Artists,
    Playlists,
    NowPlaying,
    Equalizer,
}

/// The application state, shared by every screen (`Copy`: signals are handles).
#[derive(Clone, Copy)]
pub struct App {
    pub player: Player,
    pub eq: Equalizer,
    pub page: Signal<Page>,
    /// The search box's text.
    pub query: Signal<String>,
    pub viz: Signal<VizMode>,
    /// Whether the Up Next panel is open.
    pub queue_open: Signal<bool>,
    pub focus: Focus,
}

impl App {
    /// Shows `page` (and gives the keyboard back to the shortcuts).
    pub fn open(self, page: Page) {
        self.focus.release();
        if self.page.get_untracked() != page {
            self.page.set(page);
        }
    }
}

/// The two focusable nodes and how to move the keyboard between them.
#[derive(Clone, Copy)]
pub struct Focus {
    pub root: NodeRef<Obj>,
    pub search: NodeRef<Textarea>,
}

impl Focus {
    pub fn new(cx: Scope) -> Self {
        Self {
            root: cx.node_ref(),
            search: cx.node_ref(),
        }
    }

    /// Keys go to the window's shortcuts again.
    pub fn release(self) {
        self.root.with_mut(|_, cx| {
            let n = cx.node();
            cx.engine_mut().focus(n);
        });
    }

    /// Keys go to the search field.
    pub fn search(self) {
        self.search.with_mut(|_, cx| {
            let n = cx.node();
            cx.engine_mut().focus(n);
        });
    }
}

// ---- Text ----------------------------------------------------------------------------------

/// Primary text, one line, ellipsized.
pub fn text<M>(t: impl IntoText<M>) -> WidgetView<Label> {
    label(t)
        .font(BODY)
        .text_color(design::ON_SURFACE)
        .long_mode(LongMode::Dots)
        .max_lines(1)
}

/// Secondary text, one line, ellipsized.
pub fn caption<M>(t: impl IntoText<M>) -> WidgetView<Label> {
    label(t)
        .font(CAPTION)
        .text_color(design::ON_SURFACE_MUTED)
        .long_mode(LongMode::Dots)
        .max_lines(1)
}

/// A small, spaced-out, upper-case label (section names, column headers).
pub fn overline<M>(t: impl IntoText<M>) -> WidgetView<Label> {
    label(t).font(OVERLINE).text_color(FAINT).letter_spacing(1)
}

// ---- Buttons -------------------------------------------------------------------------------

/// A flat round button showing `glyph` (a symbol), `size` px wide. Muted at rest, a soft
/// disc when pressed, the accent while `active`.
pub fn icon_button<MT, MA, R>(
    app: App,
    glyph: impl IntoText<MT>,
    size: i32,
    active: impl IntoProp<bool, MA>,
    mut on_click: impl FnMut() -> R + 'static,
) -> Flex {
    row(label(glyph).font(&fonts::MONTSERRAT_16))
        .size(size, size)
        .radius(Radius::Circle)
        .justify(MainAlign::Center)
        .align_items(CrossAlign::Center)
        .text_color(design::ON_SURFACE_MUTED)
        .scrollable(false)
        .state(State::CHECKED, active)
        .on_state(State::CHECKED, |s| s.text_color(design::PRIMARY))
        .on_state(State::PRESSED, |s| s.bg(PRESSED).text_color(design::ON_SURFACE))
        .on_press(move || app.focus.release())
        .on_click(move || {
            on_click();
        })
}

/// A pill-shaped text button: `primary` is filled with the accent, otherwise outlined.
pub fn pill<MT, R>(
    app: App,
    t: impl IntoText<MT>,
    primary: bool,
    mut on_click: impl FnMut() -> R + 'static,
) -> Flex {
    let base = row(label(t).font(BODY))
        .padding_x(theme::S4)
        .height(34)
        .radius(Radius::Circle)
        .align_items(CrossAlign::Center)
        .scrollable(false)
        .on_press(move || app.focus.release())
        .on_click(move || {
            on_click();
        });
    let base = base.on_state(State::DISABLED, |s| s.opacity(Opa::P40));
    if primary {
        base.bg(design::PRIMARY)
            .text_color(design::ON_PRIMARY)
            .on_state(State::PRESSED, |s| s.bg(theme::AMBER))
    } else {
        base.border(1, theme::LINE)
            .text_color(design::ON_SURFACE)
            .on_state(State::PRESSED, |s| s.bg(PRESSED))
    }
}

// ---- Icons ---------------------------------------------------------------------------------

/// The drawn line icons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Icon {
    #[default]
    Songs,
    Albums,
    Artists,
    Playlists,
    NowPlaying,
    Equalizer,
    Search,
}

/// What an icon canvas draws: the icon and whether it is highlighted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Glyph {
    pub icon: Icon,
    pub active: bool,
}

/// An icon view (20 × 20 by default; it scales to the node).
pub fn icon<M>(g: impl IntoProp<Glyph, M>) -> WidgetView<Canvas<Glyph>> {
    canvas(g, paint_icon).size(20, 20)
}

fn paint_icon(g: Glyph, cx: &mut DrawCx<'_, '_>) {
    let a = cx.coords();
    let s = a.width().min(a.height());
    let (ox, oy) = (a.x0 + (a.width() - s) / 2, a.y0 + (a.height() - s) / 2);
    // A point on the 20-unit design grid.
    let pt = |x: i32, y: i32| Point::new(ox + x * s / 20, oy + y * s / 20);
    let u = |v: i32| v * s / 20;
    let col = if g.active { palette::ACCENT } else { palette::MUTED };
    let w = (s / 10).max(2);
    let p = cx.painter();
    let o = Opa::COVER;
    match g.icon {
        Icon::Songs => {
            circle(p, pt(7, 15), u(3), col, o);
            line(p, pt(9, 15), pt(9, 3), w, col, o);
            line(p, pt(9, 3), pt(15, 5), w, col, o);
        }
        Icon::Albums => {
            ring(p, pt(10, 10), u(8), w, col, o);
            circle(p, pt(10, 10), u(2), col, o);
        }
        Icon::Artists => {
            ring(p, pt(10, 7), u(4), w, col, o);
            let dsc = ArcDsc {
                color: col,
                opa: o,
                width: w,
                rounded: true,
                ..ArcDsc::default()
            };
            p.arc(pt(10, 20), u(7), Angle::deg(190), Angle::deg(350), &dsc);
        }
        Icon::Playlists => {
            line(p, pt(3, 5), pt(13, 5), w, col, o);
            line(p, pt(3, 10), pt(13, 10), w, col, o);
            line(p, pt(3, 15), pt(9, 15), w, col, o);
            circle(p, pt(14, 16), u(2) + 1, col, o);
            line(p, pt(16, 16), pt(16, 8), w, col, o);
        }
        Icon::NowPlaying => {
            for (i, h) in [3, 7, 10, 6, 4].into_iter().enumerate() {
                let x = 3 + i as i32 * 35 / 10;
                line(p, pt(x, 10 - h), pt(x, 10 + h), w, col, o);
            }
        }
        Icon::Equalizer => {
            for (i, k) in [6, 13, 9].into_iter().enumerate() {
                let x = 4 + i as i32 * 6;
                line(p, pt(x, 3), pt(x, 17), (w / 2).max(1), col, Opa::P60);
                circle(p, pt(x, k), u(2) + 1, col, o);
            }
        }
        Icon::Search => {
            ring(p, pt(8, 8), u(6), w, col, o);
            line(p, pt(13, 13), pt(17, 17), w, col, o);
        }
    }
}

// ---- Sliders -------------------------------------------------------------------------------

/// Twine's slider dressed for the player: a thin rounded track, the accent fill and a small
/// white knob with a soft shadow. Not focusable, so dragging it leaves the arrow keys to the
/// shortcuts (seek and volume).
pub fn slim_slider(value: Signal<i32>, thickness: i32) -> WidgetView<Slider> {
    slider(value)
        .focusable(false)
        .bg(RAISED)
        .radius(Radius::Circle)
        .padding(0)
        .part(Part::Indicator, |s| s.bg(design::PRIMARY).radius(Radius::Circle))
        .part(Part::Knob, |s| {
            s.bg(palette::TEXT)
                .radius(Radius::Circle)
                .padding(thickness / 2 + 3)
                .shadow_width(10)
                .shadow_opacity(Opa::P50)
                .shadow_color(Color::BLACK)
        })
        .on_state(State::PRESSED, |s| {
            s.part(Part::Knob, |k| k.padding(thickness / 2 + 4).bg(Color::WHITE))
        })
}
