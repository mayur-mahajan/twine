//! Everything an application needs: `use twine_view::prelude::*;` (the `twine` facade's
//! prelude re-exports this one and adds the built-in fonts).

pub use crate::text;
pub use crate::{
    AnimController, AnyView, BuildCx, Container, Dynamic, Flex, ForEach, Grid, IntoAnyView, IntoModel,
    IntoProp, IntoText, ModalHandle, Model, Navigator, NodeRef, Prop, ScopeExt, ScreenAnim, ScreenLoad,
    TextFn, ThemeHandle, Ui, UiBuilder, View, ViewExt, ViewSeq, VirtualList, Wake, When, WhenElse,
    WidgetView, button, column, container, dynamic, flex, for_each, grid, image, label, navigator, row,
    scroll_view, spacer, stack, use_navigator, use_theme, virtual_list, when, widget_view,
};
pub use crate::{
    SpanView, animimg, arc, bar, buttonmatrix, checkbox, image_button, keyboard, led, line, line_static,
    slider, span, spangroup, spinbox, spinner, switch, textarea,
};

#[cfg(feature = "async")]
pub use crate::{AsyncUi, AsyncUiBuilder};
pub use twine_anim::{Anim, Easing, Interpolate, Repeat};
pub use twine_core::{Angle, Color, Duration, Insets, Instant, Opa, Point, Rect, Scale, Size};
pub use twine_engine::{
    BufferMode, DrawCx, Engine, Event, EventCode, EventCx, EventResult, GroupId, MeasureCx, NodeId, ObjFlags,
    Widget, WidgetClass, WidgetCx,
};
pub use twine_hal::Key;
pub use twine_image::ImageSource;
pub use twine_reactive::{
    Channel, EffectId, Memo, ReadSignal, Scope, Signal, UiWaker, WriteSignal, batch, untrack,
};
pub use twine_render::{BlendMode, BorderSide, Gradient, ShadowDsc};
pub use twine_style::{
    Align, BaseDir, Dir, FlexAlign, FlexFlow, GridAlign, GridTrack, LayoutKind, Length, Part, PropId,
    ScrollSnap, ScrollbarMode, Selector, State, Style, StyleBuf, StyleProp, StyleRef, TransitionDsc, style,
};
pub use twine_text::{Font, LongMode, TextAlign, TextDecor, symbols};
pub use twine_theme::{DefaultTheme, MonoTheme, Palette, SimpleTheme, Theme, ThemeMode};
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
