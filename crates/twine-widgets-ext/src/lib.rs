//! # twine-widgets-ext
//!
//! The complex widgets of the Twine GUI library, each a port of the LVGL widget of the same
//! name, built on the basic widgets of `twine-widgets`:
//!
//! | Widget | LVGL | Classes (theme names) |
//! |--------|------|-----------------------|
//! | [`Dropdown`](dropdown::Dropdown) | `lv_dropdown` | `dropdown`, `dropdown_list` |
//! | [`Roller`](roller::Roller) | `lv_roller` | `roller` |
//! | [`List`](list::List) | `lv_list` | `list`, `list_text`, `list_button` |
//! | [`Menu`](menu::Menu) | `lv_menu` | `menu`, `menu_page`, `menu_cont`, `menu_section`, `menu_separator`, `menu_main_container`, `menu_main_header_container`, `menu_sidebar_container`, `menu_sidebar_header_container` |
//! | [`Tabview`](tabview::Tabview) | `lv_tabview` | `tabview`, `tabview_tab_bar`, `tabview_content` |
//! | [`Tileview`](tileview::Tileview) | `lv_tileview` | `tileview`, `tileview_tile` |
//! | [`Window`](window::Window) | `lv_win` | `win`, `win_header`, `win_content` |
//! | `VectorView` (feature `vector`) | `lv_draw_vector` in a canvas | `vector_view` |
//! | [`Msgbox`](msgbox::Msgbox) | `lv_msgbox` | `msgbox`, `msgbox_backdrop`, `msgbox_header`, `msgbox_content`, `msgbox_footer`, `msgbox_header_button`, `msgbox_footer_button` |
//!
//! ## Conventions
//!
//! The conventions of `twine-widgets` apply: a module `x` has `pub struct X`, `pub static
//! X_CLASS: WidgetClass` and `pub fn create(engine, parent)`; setters take a
//! [`WidgetCx`](twine_engine::WidgetCx), are idempotent (P3) and log
//! `trace!(target: "twine::engine", "<class>#<node> set_<name>")`; the look comes from styles
//! only; invalid input logs `warn!` and is ignored (P7).
//!
//! Composite widgets (a window's header, a menu's pages) are made of child nodes of their own
//! classes, so themes style them by class name like LVGL's `lv_obj_check_type`: the theme
//! crate does not depend on this one. Popups (a dropdown's option list) live on the display's
//! top layer while open and are deleted when closed.
//!
//! ## Features
//!
//! - `std`, `log`, `defmt`: as in every Twine crate.
//! - `vector`: `vector_view::VectorView` over `twine-vector`.
//! - `bidi`, `arabic-shaping`: right-to-left text; shaped dropdown and roller options.
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::float_arithmetic))]

extern crate alloc;

pub mod dropdown;
pub mod list;
pub mod menu;
pub mod msgbox;
mod options;
pub mod prelude;
pub mod roller;
pub mod tabview;
pub mod tileview;
mod util;
#[cfg(feature = "vector")]
pub mod vector_view;
pub mod window;

pub use options::Options;
pub use util::ClassObj;
