//! The popover of a pressed [`BtnCtrl::POPOVER`](super::BtnCtrl::POPOVER) button: an enlarged
//! copy of the key, one row higher, on the display's top layer.
//!
//! LVGL draws it inside the button matrix with an extended draw area, where siblings drawn
//! later cover it and the parent clips it. Twine keeps one node per button matrix on the top
//! layer instead (created on the first press, hidden while no popover key is pressed); it is
//! not clickable, so input keeps going to the matrix.

use alloc::boxed::Box;
use alloc::string::String;

use twine_core::{Rect, Size};
use twine_engine::{DrawCx, Engine, MeasureCx, NodeId, ObjFlags, State, Widget, WidgetClass, fmt_node_id};
use twine_style::{Part, TextAlign};
use twine_text::TextLayout;

/// The class of the popover node: `"buttonmatrix_popover"`, no flags (not clickable, not
/// scrollable, not focusable).
pub static POPOVER_CLASS: WidgetClass =
    WidgetClass::new("buttonmatrix_popover").default_flags(ObjFlags::empty());

/// The popover widget: draws the owner's `Items` styles in the pressed button's state.
#[derive(Debug)]
pub(crate) struct Popover {
    owner: NodeId,
    text: String,
    state: State,
}

impl Widget for Popover {
    fn class(&self) -> &'static WidgetClass {
        &POPOVER_CLASS
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        let e = cx.engine();
        if !e.tree().contains(self.owner) {
            return;
        }
        let area = cx.coords();
        let opa = e.opa_recursive(self.owner);
        let rect = e.rect_dsc_for_state(self.owner, Part::Items, self.state, opa);
        let mut text = e.text_dsc_for_state(self.owner, Part::Items, self.state, opa);
        let mut dsc = rect.dsc();
        // Drawn in one go (LVGL `lv_draw_rect`).
        dsc.border_post = false;
        cx.painter().rect(area, &dsc);
        let mut l = TextLayout::new(&self.text, text.font);
        l.letter_space = text.letter_space;
        l.line_space = text.line_space;
        let size: Size = l.measure();
        // LVGL: centered in the grown area, then moved up by half a button (the upper half).
        let btn_h = area.height() / 2;
        let x1 = area.x0 + (area.width() - size.w) / 2;
        let y1 = area.y0 + (area.height() - size.h) / 2 - btn_h / 2;
        text.align = TextAlign::Center;
        cx.draw_text(
            Rect::new(x1, y1, x1 + size.w + 1, y1 + size.h + 1),
            &self.text,
            &text,
        );
    }

    fn covers(&self, _cx: &MeasureCx<'_>, _area: Rect) -> bool {
        false
    }

    fn text(&self) -> Option<&str> {
        Some(&self.text)
    }
}

/// Shows the popover of the button at `btn` (absolute) with `text`, drawn with the `Items`
/// styles of `state`, for `owner`; creates the
/// node on the owner display's top layer when `existing` is missing or dead. Returns the
/// popover node.
pub(crate) fn show(
    e: &mut Engine,
    owner: NodeId,
    existing: Option<NodeId>,
    btn: Rect,
    text: &str,
    state: State,
) -> Option<NodeId> {
    let node = if let Some(p) = existing.filter(|p| e.tree().contains(*p)) {
        p
    } else {
        let display = e.display_of(owner)?;
        let layer = e.top_layer(display)?;
        let p = e
            .create(
                layer,
                Box::new(Popover {
                    owner,
                    text: String::new(),
                    state: State::PRESSED,
                }),
            )
            .ok()?;
        twine_core::debug!(target: "twine::engine", "buttonmatrix#{} popover {}", fmt_node_id(owner), fmt_node_id(p));
        p
    };
    e.with_widget_mut(node, |p: &mut Popover, cx| {
        if p.text != text || p.state != state {
            p.text.clear();
            p.text.push_str(text);
            p.state = state;
            cx.invalidate();
        }
    });
    // The key grown upwards by one key height (LVGL `btn_area.y1 -= btn_height`).
    let h = btn.height();
    e.set_pos(node, btn.x0, btn.y0 - h);
    e.set_size(node, btn.width(), 2 * h);
    e.set_flag(node, ObjFlags::HIDDEN, false);
    Some(node)
}

/// Hides the popover.
pub(crate) fn hide(e: &mut Engine, popover: NodeId) {
    if e.tree().contains(popover) {
        e.set_flag(popover, ObjFlags::HIDDEN, true);
    }
}
