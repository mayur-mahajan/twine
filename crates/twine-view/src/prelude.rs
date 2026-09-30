//! Everything an application needs: `use twine_view::prelude::*;` (the `twine` facade's
//! prelude re-exports this one and adds the built-in fonts).

pub use crate::text;
pub use crate::{
    AnimController, AnyView, BuildCx, BuildError, Container, Dynamic, Flex, ForEach, Grid, IntoAnyView,
    IntoGridSpan, IntoIcon, IntoModel, IntoOptions, IntoProp, IntoText, ModalHandle, Model, ModelValue,
    Navigator, NodeRef, Prop, PropValue, ScopeExt, ScreenAnim, ScreenLoad, TextFn, ThemeHandle, Ui,
    UiBuilder, UiError, View, ViewExt, ViewSeq, VirtualList, Wake, When, WhenElse, WidgetView, button,
    column, container, dynamic, flex, for_each, grid, image, label, navigator, row, scroll_view, spacer,
    stack, use_navigator, use_theme, virtual_list, when, widget_view,
};
pub use crate::{
    Btn, MenuPageRef, MenuPageView, SpanView, TabView, TilePos, TileView, animimg, arc, bar, btn,
    buttonmatrix, checkbox, dropdown, image_button, keyboard, led, line, line_static, list, list_button,
    list_text, menu, menu_cont, menu_page, menu_section, menu_separator, msgbox, roller, slider, span,
    spangroup, spinbox, spinner, switch, tab, tabview, textarea, tile, tileview, window, window_button,
};

#[cfg(feature = "async")]
pub use crate::{AsyncUi, AsyncUiBuilder};
#[cfg(feature = "vector")]
pub use crate::{VectorCanvas, vector_canvas};
pub use twine_anim::{Anim, Easing, Interpolate, Repeat};
pub use twine_core::fault::{FaultCounts, FaultKind, Faults};
pub use twine_core::{
    Angle, AngularSpeed, Color, Duration, Fraction, Insets, Instant, Opa, Point, Rect, Scale, Size,
};
pub use twine_engine::{
    BufferMode, DEAD_NODE, DisplayHealth, DisplayState, DrawCx, Engine, Event, EventCode, EventCx,
    EventResult, FaultHook, FaultRecord, FlushPolicy, GroupId, MeasureCx, NodeId, ObjFlags, Widget,
    WidgetClass, WidgetCx,
};
pub use twine_hal::Key;
pub use twine_image::ImageSource;
pub use twine_reactive::{
    Channel, EffectId, Memo, ReadSignal, Scope, Signal, UiWaker, WriteSignal, batch, untrack,
};
pub use twine_render::{BlendMode, BorderSide, Gradient, ShadowDsc};
pub use twine_style::{
    Align, Anchor, Axis, BaseDir, CrossAlign, FlexDirection, FlexFlow, GridAlign, GridSpan, GridTrack,
    LayoutKind, Length, MainAlign, Part, PropId, Radius, ScrollSnap, ScrollbarMode, Selector, Side, Sides,
    State, Style, StyleBuf, StyleProp, StyleRef, TransitionDsc, grid_tracks, style,
};
pub use twine_text::{Font, LongMode, Symbol, TextAlign, TextDecor};
pub use twine_theme::{DefaultTheme, MonoTheme, Palette, SimpleTheme, Theme, ThemeMode};
#[cfg(feature = "vector")]
pub use twine_vector::{FxPoint, Path, VectorDsc, VectorScene};
pub use twine_widgets::Orientation;
pub use twine_widgets::arc::ArcMode;
pub use twine_widgets::bar::BarMode;
pub use twine_widgets::button::Button;
pub use twine_widgets::buttonmatrix::BtnCtrl;
pub use twine_widgets::image::ImageAlign;
pub use twine_widgets::image_button::ImageButtonState;
pub use twine_widgets::keyboard::KeyboardMode;
pub use twine_widgets::label::Label;
pub use twine_widgets::slider::SliderMode;
pub use twine_widgets::spangroup::{SpanMode, SpanOverflow};
pub use twine_widgets::spinbox::Spinbox;
pub use twine_widgets::textarea::Textarea;
pub use twine_widgets_ext::dropdown::Dropdown;
pub use twine_widgets_ext::list::List;
pub use twine_widgets_ext::menu::{Menu, MenuHeaderMode, MenuPage};
pub use twine_widgets_ext::msgbox::Msgbox;
pub use twine_widgets_ext::roller::{Roller, RollerMode};
pub use twine_widgets_ext::tabview::Tabview;
pub use twine_widgets_ext::tileview::{Tile, Tileview};
pub use twine_widgets_ext::window::Window;
