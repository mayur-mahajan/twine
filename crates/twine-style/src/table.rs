//! The property and shorthand tables: the single source of the style vocabulary.
//!
//! Every style property has **one name**, used unchanged as the `style!` key, the
//! [`StyleBuf`](crate::StyleBuf) builder method and the `twine-view` modifier; its
//! [`PropId`](crate::PropId) / [`StyleProp`](crate::StyleProp) variant is the same name in
//! `PascalCase`. The tables below are data only: [`__prop_table!`] and [`__shorthand_table!`]
//! hand the rows to a callback macro (`define_props` and `define_shorthands` in this crate, the
//! modifier generator in `twine-view`), so the three APIs cannot drift apart.
//!
//! # Vocabulary
//!
//! Names are plain Rust/CSS words, not LVGL abbreviations. The previous Twine key and the LVGL
//! constant of every property are kept as `#[doc(alias)]`es (and listed in `PROPERTIES.md`):
//!
//! | Rule | Examples (old key → name) |
//! |------|---------------------------|
//! | `pad_*` → `padding_*`; row/column padding is a gap | `pad_top` → `padding_top`, `pad_row` → `row_gap`, `pad_column` → `column_gap` |
//! | `*_opa` → `*_opacity` | `bg_opa` → `bg_opacity`, `text_opa` → `text_opacity` |
//! | Group opacity is `opacity`; the per-part factor is `part_opacity` | `opa_layered` → `opacity`, `opa` → `part_opacity` |
//! | Text properties use CSS names | `text_font` → `font`, `text_letter_space` → `letter_spacing`, `text_line_space` → `line_spacing`, `text_decor` → `text_decoration` |
//! | `*_src`, `*_dsc`, `*_dsc_array` suffixes are dropped | `bg_image_src` → `bg_image`, `color_filter_dsc` → `color_filter`, `grid_column_dsc_array` → `grid_column_tracks` |
//! | `grad` → `gradient`; gradient stops are start/end | `bg_grad` → `bg_gradient`, `bg_main_stop` → `bg_gradient_start`, `bg_grad_opa` → `bg_gradient_end_opacity` |
//! | Flex placement is alignment | `flex_main_place` → `flex_main_align` |
//! | Misc. | `outline_pad` → `outline_offset`, `border_post` → `border_above_children`, `image_colorkey` → `image_color_key`, `text_outline_stroke_*` → `text_outline_*`, `grid_cell_column_pos` → `grid_cell_column` |
//!
//! Shorthands (`padding`, `padding_x`, `gap`, `size`, `bg`, `border`, …) set several
//! properties at once and exist under the same name in all three APIs.
//!
//! # Row formats
//!
//! Property rows:
//!
//! ```text
//! /// docs
//! Name(key): kind [payload type] [LVGL flags] DEFAULT ["old key" "LV_STYLE_…"];
//! ```
//!
//! `kind` is how `style!` converts values: `len` (integers become `Length::Px`, any `Length`
//! such as `Length::dp(8)` stays), `radius` (integers become `Radius::Px`, a `Radius` stays),
//! `dur` (a `Duration` becomes `DurationMs`) or
//! `val` (as is: typed values only, e.g. `Opa::pct(50)`, `Angle::deg(30)`, `Scale::pct(98)`).
//! The payload type is written with the names of `twine_style::__private` and resolved by
//! [`__prop_ty!`]; `DEFAULT` names a constant of `twine_style::__private::defaults`.
//!
//! Shorthand rows:
//!
//! ```text
//! /// docs
//! name(param: kind<G>? [type] => Prop, Prop(.field), Prop(px .field), Prop = CONST; param: …)
//!     ["old key"] { /// doctest };
//! ```
//!
//! Each parameter sets the listed properties: `Prop` to the parameter, `Prop(.field)` to a field
//! of it, `Prop(px .field)` to `Length::Px` of a field, `Prop = CONST` to a constant of
//! `twine_style::__private`. `len` parameters name the generic type used by the view modifier;
//! `span` parameters (a `GridSpan`) take an index or a range in the view modifier
//! (`IntoGridSpan`) and the builder (`Into<GridSpan>`), a `GridSpan` in `style!`.
//! The block after the aliases is the doctest of the `StyleBuf` method.

/// Hands the style property table to the callback macro `$cb` as
/// `$cb! { [args] rows… }` (see the module documentation of `table.rs` for the row format).
#[doc(hidden)]
#[macro_export]
macro_rules! __prop_table {
    ($cb:ident $($args:tt)*) => {
        $cb! {
            [$($args)*]
            // ---- Size and position -----------------------------------------------------------
            /// Width (`LV_STYLE_WIDTH`). Default `Content` (LVGL: widget class dependent).
            Width(width): len [Length] [LAYOUT] CONTENT ["LV_STYLE_WIDTH"];
            /// Minimal width (`LV_STYLE_MIN_WIDTH`); percent of the parent's content width.
            MinWidth(min_width): len [Length] [LAYOUT] PX0 ["LV_STYLE_MIN_WIDTH"];
            /// Maximal width (`LV_STYLE_MAX_WIDTH`).
            MaxWidth(max_width): len [Length] [LAYOUT] COORD_MAX_PX ["LV_STYLE_MAX_WIDTH"];
            /// Height (`LV_STYLE_HEIGHT`). Default `Content` (LVGL: widget class dependent).
            Height(height): len [Length] [LAYOUT] CONTENT ["LV_STYLE_HEIGHT"];
            /// Minimal height (`LV_STYLE_MIN_HEIGHT`).
            MinHeight(min_height): len [Length] [LAYOUT] PX0 ["LV_STYLE_MIN_HEIGHT"];
            /// Maximal height (`LV_STYLE_MAX_HEIGHT`).
            MaxHeight(max_height): len [Length] [LAYOUT] COORD_MAX_PX ["LV_STYLE_MAX_HEIGHT"];
            /// X position relative to the `Align` reference (`LV_STYLE_X`).
            X(x): len [Length] [LAYOUT] PX0 ["LV_STYLE_X"];
            /// Y position relative to the `Align` reference (`LV_STYLE_Y`).
            Y(y): len [Length] [LAYOUT] PX0 ["LV_STYLE_Y"];
            /// Alignment in the parent (`LV_STYLE_ALIGN`).
            Align(align): val [Align] [LAYOUT] ALIGN_DEFAULT ["LV_STYLE_ALIGN"];
            /// Draws the object wider on both sides; percent of its width (`LV_STYLE_TRANSFORM_WIDTH`).
            TransformWidth(transform_width): len [Length] [EXT_DRAW TRANSFORM] PX0 ["LV_STYLE_TRANSFORM_WIDTH"];
            /// Draws the object taller on both sides (`LV_STYLE_TRANSFORM_HEIGHT`).
            TransformHeight(transform_height): len [Length] [EXT_DRAW TRANSFORM] PX0 ["LV_STYLE_TRANSFORM_HEIGHT"];
            /// Moves the object after layout; percent of its width (`LV_STYLE_TRANSLATE_X`).
            TranslateX(translate_x): len [Length] [LAYOUT PARENT_LAYOUT] PX0 ["LV_STYLE_TRANSLATE_X"];
            /// Moves the object after layout; percent of its height (`LV_STYLE_TRANSLATE_Y`).
            TranslateY(translate_y): len [Length] [LAYOUT PARENT_LAYOUT] PX0 ["LV_STYLE_TRANSLATE_Y"];
            /// Moves radial items (e.g. scale labels) outward (`LV_STYLE_TRANSLATE_RADIAL`).
            TranslateRadial(translate_radial): val [i32] [] INT0 ["LV_STYLE_TRANSLATE_RADIAL"];
            /// Horizontal zoom (`LV_STYLE_TRANSFORM_SCALE_X`).
            TransformScaleX(transform_scale_x): val [Scale] [EXT_DRAW LAYER TRANSFORM] SCALE_ONE ["LV_STYLE_TRANSFORM_SCALE_X"];
            /// Vertical zoom (`LV_STYLE_TRANSFORM_SCALE_Y`).
            TransformScaleY(transform_scale_y): val [Scale] [EXT_DRAW LAYER TRANSFORM] SCALE_ONE ["LV_STYLE_TRANSFORM_SCALE_Y"];
            /// Rotation (`LV_STYLE_TRANSFORM_ROTATION`).
            TransformRotation(transform_rotation): val [Angle] [EXT_DRAW LAYER TRANSFORM] ANGLE0 ["LV_STYLE_TRANSFORM_ROTATION"];
            /// Pivot of rotation/zoom from the left edge; percent of the width (`LV_STYLE_TRANSFORM_PIVOT_X`).
            TransformPivotX(transform_pivot_x): len [Length] [] PX0 ["LV_STYLE_TRANSFORM_PIVOT_X"];
            /// Pivot of rotation/zoom from the top edge (`LV_STYLE_TRANSFORM_PIVOT_Y`).
            TransformPivotY(transform_pivot_y): len [Length] [] PX0 ["LV_STYLE_TRANSFORM_PIVOT_Y"];
            /// Horizontal skew (`LV_STYLE_TRANSFORM_SKEW_X`).
            TransformSkewX(transform_skew_x): val [Angle] [EXT_DRAW LAYER TRANSFORM] ANGLE0 ["LV_STYLE_TRANSFORM_SKEW_X"];
            /// Vertical skew (`LV_STYLE_TRANSFORM_SKEW_Y`).
            TransformSkewY(transform_skew_y): val [Angle] [EXT_DRAW LAYER TRANSFORM] ANGLE0 ["LV_STYLE_TRANSFORM_SKEW_Y"];
            // ---- Padding and margin ----------------------------------------------------------
            /// Top padding (`LV_STYLE_PAD_TOP`).
            PaddingTop(padding_top): len [Length] [EXT_DRAW LAYOUT] PX0 ["pad_top" "LV_STYLE_PAD_TOP"];
            /// Bottom padding (`LV_STYLE_PAD_BOTTOM`).
            PaddingBottom(padding_bottom): len [Length] [EXT_DRAW LAYOUT] PX0 ["pad_bottom" "LV_STYLE_PAD_BOTTOM"];
            /// Left padding (`LV_STYLE_PAD_LEFT`).
            PaddingLeft(padding_left): len [Length] [EXT_DRAW LAYOUT] PX0 ["pad_left" "LV_STYLE_PAD_LEFT"];
            /// Right padding (`LV_STYLE_PAD_RIGHT`).
            PaddingRight(padding_right): len [Length] [EXT_DRAW LAYOUT] PX0 ["pad_right" "LV_STYLE_PAD_RIGHT"];
            /// Gap between rows of a flex or grid container (`LV_STYLE_PAD_ROW`).
            RowGap(row_gap): len [Length] [EXT_DRAW LAYOUT] PX0 ["pad_row" "LV_STYLE_PAD_ROW"];
            /// Gap between columns of a flex or grid container (`LV_STYLE_PAD_COLUMN`).
            ColumnGap(column_gap): len [Length] [EXT_DRAW LAYOUT] PX0 ["pad_column" "LV_STYLE_PAD_COLUMN"];
            /// Radial padding of radial items (`LV_STYLE_PAD_RADIAL`).
            PaddingRadial(padding_radial): val [i32] [] INT0 ["pad_radial" "LV_STYLE_PAD_RADIAL"];
            /// Top margin, used by flex/grid placement (`LV_STYLE_MARGIN_TOP`).
            MarginTop(margin_top): len [Length] [EXT_DRAW LAYOUT] PX0 ["LV_STYLE_MARGIN_TOP"];
            /// Bottom margin (`LV_STYLE_MARGIN_BOTTOM`).
            MarginBottom(margin_bottom): len [Length] [EXT_DRAW LAYOUT] PX0 ["LV_STYLE_MARGIN_BOTTOM"];
            /// Left margin (`LV_STYLE_MARGIN_LEFT`).
            MarginLeft(margin_left): len [Length] [EXT_DRAW LAYOUT] PX0 ["LV_STYLE_MARGIN_LEFT"];
            /// Right margin (`LV_STYLE_MARGIN_RIGHT`).
            MarginRight(margin_right): len [Length] [EXT_DRAW LAYOUT] PX0 ["LV_STYLE_MARGIN_RIGHT"];
            // ---- Background ------------------------------------------------------------------
            /// Background color (`LV_STYLE_BG_COLOR`); transparent until `bg_opacity` is set
            /// (the `bg` shorthand sets both).
            BgColor(bg_color): val [Color] [] WHITE ["LV_STYLE_BG_COLOR"];
            /// Background opacity (`LV_STYLE_BG_OPA`).
            BgOpacity(bg_opacity): val [Opa] [] TRANSP ["bg_opa" "LV_STYLE_BG_OPA"];
            /// Gradient end color for `Ver`/`Hor` gradients (`LV_STYLE_BG_GRAD_COLOR`).
            BgGradientColor(bg_gradient_color): val [Color] [] BLACK ["bg_grad_color" "LV_STYLE_BG_GRAD_COLOR"];
            /// Simple gradient direction (`LV_STYLE_BG_GRAD_DIR`).
            BgGradientDir(bg_gradient_dir): val [GradDir] [] GRAD_DIR_NONE ["bg_grad_dir" "LV_STYLE_BG_GRAD_DIR"];
            /// Where the gradient starts along the object (`LV_STYLE_BG_MAIN_STOP`).
            BgGradientStart(bg_gradient_start): val [Fraction] [] INT0 ["bg_main_stop" "LV_STYLE_BG_MAIN_STOP"];
            /// Where the gradient ends along the object (`LV_STYLE_BG_GRAD_STOP`).
            BgGradientEnd(bg_gradient_end): val [Fraction] [] INT255 ["bg_grad_stop" "LV_STYLE_BG_GRAD_STOP"];
            /// Opacity of the gradient's start color (`LV_STYLE_BG_MAIN_OPA`).
            BgGradientStartOpacity(bg_gradient_start_opacity): val [Opa] [] COVER ["bg_main_opa" "LV_STYLE_BG_MAIN_OPA"];
            /// Opacity of the gradient's end color (`LV_STYLE_BG_GRAD_OPA`).
            BgGradientEndOpacity(bg_gradient_end_opacity): val [Opa] [] COVER ["bg_grad_opa" "LV_STYLE_BG_GRAD_OPA"];
            /// Full gradient descriptor; overrides the simple gradient (`LV_STYLE_BG_GRAD`).
            BgGradient(bg_gradient): val [&'static Gradient] [] NONE ["bg_grad" "LV_STYLE_BG_GRAD"];
            /// Background image or symbol (`LV_STYLE_BG_IMAGE_SRC`).
            BgImage(bg_image): val [&'static ImageSource] [EXT_DRAW] NONE ["bg_image_src" "LV_STYLE_BG_IMAGE_SRC"];
            /// Background image opacity (`LV_STYLE_BG_IMAGE_OPA`).
            BgImageOpacity(bg_image_opacity): val [Opa] [] COVER ["bg_image_opa" "LV_STYLE_BG_IMAGE_OPA"];
            /// Background image recolor (`LV_STYLE_BG_IMAGE_RECOLOR`).
            BgImageRecolor(bg_image_recolor): val [Color] [] BLACK ["LV_STYLE_BG_IMAGE_RECOLOR"];
            /// Background image recolor intensity (`LV_STYLE_BG_IMAGE_RECOLOR_OPA`).
            BgImageRecolorOpacity(bg_image_recolor_opacity): val [Opa] [] TRANSP ["bg_image_recolor_opa" "LV_STYLE_BG_IMAGE_RECOLOR_OPA"];
            /// Tile the background image (`LV_STYLE_BG_IMAGE_TILED`).
            BgImageTiled(bg_image_tiled): val [bool] [] FALSE ["LV_STYLE_BG_IMAGE_TILED"];
            // ---- Border ----------------------------------------------------------------------
            /// Border color (`LV_STYLE_BORDER_COLOR`).
            BorderColor(border_color): val [Color] [] BLACK ["LV_STYLE_BORDER_COLOR"];
            /// Border opacity (`LV_STYLE_BORDER_OPA`).
            BorderOpacity(border_opacity): val [Opa] [] COVER ["border_opa" "LV_STYLE_BORDER_OPA"];
            /// Border width (`LV_STYLE_BORDER_WIDTH`).
            BorderWidth(border_width): len [Length] [LAYOUT] PX0 ["LV_STYLE_BORDER_WIDTH"];
            /// Which sides get a border (`LV_STYLE_BORDER_SIDE`).
            BorderSide(border_side): val [BorderSide] [] BORDER_SIDE_FULL ["LV_STYLE_BORDER_SIDE"];
            /// Draw the border after (above) the children (`LV_STYLE_BORDER_POST`).
            BorderAboveChildren(border_above_children): val [bool] [] FALSE ["border_post" "LV_STYLE_BORDER_POST"];
            // ---- Outline ---------------------------------------------------------------------
            /// Outline width (`LV_STYLE_OUTLINE_WIDTH`).
            OutlineWidth(outline_width): val [i32] [EXT_DRAW] INT0 ["LV_STYLE_OUTLINE_WIDTH"];
            /// Outline color (`LV_STYLE_OUTLINE_COLOR`).
            OutlineColor(outline_color): val [Color] [] BLACK ["LV_STYLE_OUTLINE_COLOR"];
            /// Outline opacity (`LV_STYLE_OUTLINE_OPA`).
            OutlineOpacity(outline_opacity): val [Opa] [EXT_DRAW] COVER ["outline_opa" "LV_STYLE_OUTLINE_OPA"];
            /// Gap between the object and the outline (`LV_STYLE_OUTLINE_PAD`).
            OutlineOffset(outline_offset): val [i32] [EXT_DRAW] INT0 ["outline_pad" "LV_STYLE_OUTLINE_PAD"];
            // ---- Shadow ----------------------------------------------------------------------
            /// Shadow blur width (`LV_STYLE_SHADOW_WIDTH`).
            ShadowWidth(shadow_width): val [i32] [EXT_DRAW] INT0 ["LV_STYLE_SHADOW_WIDTH"];
            /// Shadow horizontal offset (`LV_STYLE_SHADOW_OFFSET_X`).
            ShadowOffsetX(shadow_offset_x): val [i32] [EXT_DRAW] INT0 ["LV_STYLE_SHADOW_OFFSET_X"];
            /// Shadow vertical offset (`LV_STYLE_SHADOW_OFFSET_Y`).
            ShadowOffsetY(shadow_offset_y): val [i32] [EXT_DRAW] INT0 ["LV_STYLE_SHADOW_OFFSET_Y"];
            /// Shadow spread (`LV_STYLE_SHADOW_SPREAD`).
            ShadowSpread(shadow_spread): val [i32] [EXT_DRAW] INT0 ["LV_STYLE_SHADOW_SPREAD"];
            /// Shadow color (`LV_STYLE_SHADOW_COLOR`).
            ShadowColor(shadow_color): val [Color] [] BLACK ["LV_STYLE_SHADOW_COLOR"];
            /// Shadow opacity (`LV_STYLE_SHADOW_OPA`).
            ShadowOpacity(shadow_opacity): val [Opa] [EXT_DRAW] COVER ["shadow_opa" "LV_STYLE_SHADOW_OPA"];
            // ---- Drop shadow (shadow of the drawn content) -----------------------------------
            /// Drop shadow blur radius (`LV_STYLE_DROP_SHADOW_RADIUS`).
            DropShadowRadius(drop_shadow_radius): val [i32] [EXT_DRAW] INT0 ["LV_STYLE_DROP_SHADOW_RADIUS"];
            /// Drop shadow horizontal offset (`LV_STYLE_DROP_SHADOW_OFFSET_X`).
            DropShadowOffsetX(drop_shadow_offset_x): val [i32] [EXT_DRAW] INT0 ["LV_STYLE_DROP_SHADOW_OFFSET_X"];
            /// Drop shadow vertical offset (`LV_STYLE_DROP_SHADOW_OFFSET_Y`).
            DropShadowOffsetY(drop_shadow_offset_y): val [i32] [EXT_DRAW] INT0 ["LV_STYLE_DROP_SHADOW_OFFSET_Y"];
            /// Drop shadow color (`LV_STYLE_DROP_SHADOW_COLOR`).
            DropShadowColor(drop_shadow_color): val [Color] [] BLACK ["LV_STYLE_DROP_SHADOW_COLOR"];
            /// Drop shadow opacity (`LV_STYLE_DROP_SHADOW_OPA`).
            DropShadowOpacity(drop_shadow_opacity): val [Opa] [EXT_DRAW] TRANSP ["drop_shadow_opa" "LV_STYLE_DROP_SHADOW_OPA"];
            /// Drop shadow blur quality (`LV_STYLE_DROP_SHADOW_QUALITY`).
            DropShadowQuality(drop_shadow_quality): val [BlurQuality] [] BLUR_PRECISION ["LV_STYLE_DROP_SHADOW_QUALITY"];
            // ---- Blur ------------------------------------------------------------------------
            /// Blur radius of the part (`LV_STYLE_BLUR_RADIUS`).
            BlurRadius(blur_radius): val [i32] [] INT0 ["LV_STYLE_BLUR_RADIUS"];
            /// Blur what is behind the part instead of the part itself (`LV_STYLE_BLUR_BACKDROP`).
            BlurBackdrop(blur_backdrop): val [bool] [] FALSE ["LV_STYLE_BLUR_BACKDROP"];
            /// Blur quality (`LV_STYLE_BLUR_QUALITY`).
            BlurQuality(blur_quality): val [BlurQuality] [] BLUR_AUTO ["LV_STYLE_BLUR_QUALITY"];
            // ---- Image -----------------------------------------------------------------------
            /// Image opacity (`LV_STYLE_IMAGE_OPA`).
            ImageOpacity(image_opacity): val [Opa] [] COVER ["image_opa" "LV_STYLE_IMAGE_OPA"];
            /// Image recolor (`LV_STYLE_IMAGE_RECOLOR`).
            ImageRecolor(image_recolor): val [Color] [] BLACK ["LV_STYLE_IMAGE_RECOLOR"];
            /// Image recolor intensity (`LV_STYLE_IMAGE_RECOLOR_OPA`).
            ImageRecolorOpacity(image_recolor_opacity): val [Opa] [] TRANSP ["image_recolor_opa" "LV_STYLE_IMAGE_RECOLOR_OPA"];
            /// Colors made transparent in images (`LV_STYLE_IMAGE_COLORKEY`).
            ImageColorKey(image_color_key): val [&'static ImageColorkey] [] NONE ["image_colorkey" "LV_STYLE_IMAGE_COLORKEY"];
            // ---- Line ------------------------------------------------------------------------
            /// Line width (`LV_STYLE_LINE_WIDTH`).
            LineWidth(line_width): val [i32] [EXT_DRAW] INT0 ["LV_STYLE_LINE_WIDTH"];
            /// Dash length (`LV_STYLE_LINE_DASH_WIDTH`).
            LineDashWidth(line_dash_width): val [i32] [] INT0 ["LV_STYLE_LINE_DASH_WIDTH"];
            /// Gap between dashes (`LV_STYLE_LINE_DASH_GAP`).
            LineDashGap(line_dash_gap): val [i32] [] INT0 ["LV_STYLE_LINE_DASH_GAP"];
            /// Rounded line ends (`LV_STYLE_LINE_ROUNDED`).
            LineRounded(line_rounded): val [bool] [] FALSE ["LV_STYLE_LINE_ROUNDED"];
            /// Line color (`LV_STYLE_LINE_COLOR`).
            LineColor(line_color): val [Color] [] BLACK ["LV_STYLE_LINE_COLOR"];
            /// Line opacity (`LV_STYLE_LINE_OPA`).
            LineOpacity(line_opacity): val [Opa] [] COVER ["line_opa" "LV_STYLE_LINE_OPA"];
            // ---- Arc -------------------------------------------------------------------------
            /// Arc width (`LV_STYLE_ARC_WIDTH`).
            ArcWidth(arc_width): val [i32] [EXT_DRAW] INT0 ["LV_STYLE_ARC_WIDTH"];
            /// Rounded arc ends (`LV_STYLE_ARC_ROUNDED`).
            ArcRounded(arc_rounded): val [bool] [] FALSE ["LV_STYLE_ARC_ROUNDED"];
            /// Arc color (`LV_STYLE_ARC_COLOR`).
            ArcColor(arc_color): val [Color] [] BLACK ["LV_STYLE_ARC_COLOR"];
            /// Arc opacity (`LV_STYLE_ARC_OPA`).
            ArcOpacity(arc_opacity): val [Opa] [] COVER ["arc_opa" "LV_STYLE_ARC_OPA"];
            /// Image drawn along the arc (`LV_STYLE_ARC_IMAGE_SRC`).
            ArcImage(arc_image): val [&'static ImageSource] [] NONE ["arc_image_src" "LV_STYLE_ARC_IMAGE_SRC"];
            // ---- Text ------------------------------------------------------------------------
            /// Text color, inherited (`LV_STYLE_TEXT_COLOR`).
            TextColor(text_color): val [Color] [INHERITABLE] BLACK ["LV_STYLE_TEXT_COLOR"];
            /// Text opacity, inherited (`LV_STYLE_TEXT_OPA`).
            TextOpacity(text_opacity): val [Opa] [INHERITABLE] COVER ["text_opa" "LV_STYLE_TEXT_OPA"];
            /// Font, inherited (`LV_STYLE_TEXT_FONT`); the default comes from `StyleDefaults::font`.
            Font(font): val [&'static Font] [INHERITABLE LAYOUT] FONT_EMPTY ["text_font" "LV_STYLE_TEXT_FONT"];
            /// Extra space between letters, inherited (`LV_STYLE_TEXT_LETTER_SPACE`).
            LetterSpacing(letter_spacing): val [i32] [INHERITABLE LAYOUT] INT0 ["text_letter_space" "letter_space" "LV_STYLE_TEXT_LETTER_SPACE"];
            /// Extra space between lines, inherited (`LV_STYLE_TEXT_LINE_SPACE`).
            LineSpacing(line_spacing): val [i32] [INHERITABLE LAYOUT] INT0 ["text_line_space" "line_space" "LV_STYLE_TEXT_LINE_SPACE"];
            /// Underline/strikethrough, inherited (`LV_STYLE_TEXT_DECOR`).
            TextDecoration(text_decoration): val [TextDecor] [INHERITABLE] TEXT_DECOR_NONE ["text_decor" "LV_STYLE_TEXT_DECOR"];
            /// Horizontal text alignment, inherited (`LV_STYLE_TEXT_ALIGN`).
            TextAlign(text_align): val [TextAlign] [INHERITABLE LAYOUT] TEXT_ALIGN_AUTO ["LV_STYLE_TEXT_ALIGN"];
            /// Text outline color (`LV_STYLE_TEXT_OUTLINE_STROKE_COLOR`).
            TextOutlineColor(text_outline_color): val [Color] [] BLACK ["text_outline_stroke_color" "LV_STYLE_TEXT_OUTLINE_STROKE_COLOR"];
            /// Text outline width (`LV_STYLE_TEXT_OUTLINE_STROKE_WIDTH`).
            TextOutlineWidth(text_outline_width): val [i32] [] INT0 ["text_outline_stroke_width" "LV_STYLE_TEXT_OUTLINE_STROKE_WIDTH"];
            /// Text outline opacity (`LV_STYLE_TEXT_OUTLINE_STROKE_OPA`).
            TextOutlineOpacity(text_outline_opacity): val [Opa] [] TRANSP ["text_outline_stroke_opa" "LV_STYLE_TEXT_OUTLINE_STROKE_OPA"];
            /// Trims the space above/below text by font metrics, inherited (`LV_STYLE_TEXT_LEADING_TRIM`).
            TextLeadingTrim(text_leading_trim): val [TextLeadingTrim] [INHERITABLE LAYOUT] LEADING_TRIM_NONE ["LV_STYLE_TEXT_LEADING_TRIM"];
            // ---- Miscellaneous ---------------------------------------------------------------
            /// Generic length, e.g. of scale ticks (`LV_STYLE_LENGTH`).
            Length(length): val [i32] [EXT_DRAW] INT0 ["LV_STYLE_LENGTH"];
            /// Corner radius; `Radius::Circle` for fully round (`LV_STYLE_RADIUS`).
            Radius(radius): radius [Radius] [] PX0 ["LV_STYLE_RADIUS"];
            /// Offset of radial items (`LV_STYLE_RADIAL_OFFSET`).
            RadialOffset(radial_offset): val [i32] [] INT0 ["LV_STYLE_RADIAL_OFFSET"];
            /// Clip children to the rounded corners (`LV_STYLE_CLIP_CORNER`).
            ClipCorner(clip_corner): val [bool] [] FALSE ["LV_STYLE_CLIP_CORNER"];
            /// Opacity factor of the part, multiplied into everything it draws, without a layer
            /// (`LV_STYLE_OPA`). Cheaper than `opacity` but overlapping children
            /// show through each other.
            PartOpacity(part_opacity): val [Opa] [] COVER ["opa" "LV_STYLE_OPA"];
            /// Opacity of the object and its children, rendered as one layer
            /// (`LV_STYLE_OPA_LAYERED`).
            Opacity(opacity): val [Opa] [LAYER] COVER ["opa_layered" "LV_STYLE_OPA_LAYERED"];
            /// Color filter, inherited (`LV_STYLE_COLOR_FILTER_DSC`).
            ColorFilter(color_filter): val [&'static ColorFilter] [INHERITABLE] NONE ["color_filter_dsc" "LV_STYLE_COLOR_FILTER_DSC"];
            /// Color filter intensity, inherited (`LV_STYLE_COLOR_FILTER_OPA`).
            ColorFilterOpacity(color_filter_opacity): val [Opa] [INHERITABLE] TRANSP ["color_filter_opa" "LV_STYLE_COLOR_FILTER_OPA"];
            /// Animation template used by some widgets (`LV_STYLE_ANIM`).
            Anim(anim): val [&'static AnimTemplate] [] NONE ["LV_STYLE_ANIM"];
            /// Animation duration used by some widgets (`LV_STYLE_ANIM_DURATION`).
            AnimDuration(anim_duration): dur [DurationMs] [] INT0 ["LV_STYLE_ANIM_DURATION"];
            /// Transitions to run when entering the state (`LV_STYLE_TRANSITION`).
            Transition(transition): val [&'static TransitionDsc] [] NONE ["LV_STYLE_TRANSITION"];
            /// How the part blends with what is below (`LV_STYLE_BLEND_MODE`).
            BlendMode(blend_mode): val [BlendMode] [LAYER] BLEND_NORMAL ["LV_STYLE_BLEND_MODE"];
            /// Layout of the children (`LV_STYLE_LAYOUT`).
            Layout(layout): val [LayoutKind] [LAYOUT] LAYOUT_NONE ["LV_STYLE_LAYOUT"];
            /// Base text direction, inherited (`LV_STYLE_BASE_DIR`).
            BaseDir(base_dir): val [BaseDir] [INHERITABLE LAYOUT] BASE_DIR_LTR ["LV_STYLE_BASE_DIR"];
            /// A8/L8 image masking the object (`LV_STYLE_BITMAP_MASK_SRC`).
            BitmapMask(bitmap_mask): val [&'static ImageSource] [LAYER] NONE ["bitmap_mask_src" "LV_STYLE_BITMAP_MASK_SRC"];
            /// Recolor of everything the part draws (`LV_STYLE_RECOLOR`).
            Recolor(recolor): val [Color] [] BLACK ["LV_STYLE_RECOLOR"];
            /// Recolor intensity (`LV_STYLE_RECOLOR_OPA`).
            RecolorOpacity(recolor_opacity): val [Opa] [] TRANSP ["recolor_opa" "LV_STYLE_RECOLOR_OPA"];
            /// Encoder rotation multiplier (`LV_STYLE_ROTARY_SENSITIVITY`).
            RotarySensitivity(rotary_sensitivity): val [Scale] [] SCALE_ONE ["LV_STYLE_ROTARY_SENSITIVITY"];
            // ---- Flex ------------------------------------------------------------------------
            /// Flex direction, wrapping and order, e.g. `FlexFlow::COLUMN.wrap(true)`
            /// (`LV_STYLE_FLEX_FLOW`).
            FlexFlow(flex_flow): val [FlexFlow] [LAYOUT] FLEX_ROW ["LV_STYLE_FLEX_FLOW"];
            /// Placement of the items on the main axis (`LV_STYLE_FLEX_MAIN_PLACE`).
            FlexMainAlign(flex_main_align): val [MainAlign] [LAYOUT] FLEX_START ["flex_main_place" "LV_STYLE_FLEX_MAIN_PLACE"];
            /// Placement of the items across the main axis, in their track
            /// (`LV_STYLE_FLEX_CROSS_PLACE`).
            FlexCrossAlign(flex_cross_align): val [CrossAlign] [LAYOUT] CROSS_START ["flex_cross_place" "LV_STYLE_FLEX_CROSS_PLACE"];
            /// Placement of the tracks of a wrapping container (`LV_STYLE_FLEX_TRACK_PLACE`).
            FlexTrackAlign(flex_track_align): val [MainAlign] [LAYOUT] FLEX_START ["flex_track_place" "LV_STYLE_FLEX_TRACK_PLACE"];
            /// Weight of a flex item's share of the free main-axis space: items share it in
            /// proportion to their weights (`1` and `3` get a quarter and three quarters);
            /// 0 = the item keeps its own size (`LV_STYLE_FLEX_GROW`).
            FlexGrow(flex_grow): val [u16] [LAYOUT] INT0 ["LV_STYLE_FLEX_GROW"];
            // ---- Grid ------------------------------------------------------------------------
            /// Column template (`LV_STYLE_GRID_COLUMN_DSC_ARRAY`).
            GridColumnTracks(grid_column_tracks): val [&'static [GridTrack]] [LAYOUT] NONE ["grid_column_dsc_array" "LV_STYLE_GRID_COLUMN_DSC_ARRAY"];
            /// Row template (`LV_STYLE_GRID_ROW_DSC_ARRAY`).
            GridRowTracks(grid_row_tracks): val [&'static [GridTrack]] [LAYOUT] NONE ["grid_row_dsc_array" "LV_STYLE_GRID_ROW_DSC_ARRAY"];
            /// Column track alignment (`LV_STYLE_GRID_COLUMN_ALIGN`).
            GridColumnAlign(grid_column_align): val [GridAlign] [LAYOUT] GRID_START ["LV_STYLE_GRID_COLUMN_ALIGN"];
            /// Row track alignment (`LV_STYLE_GRID_ROW_ALIGN`).
            GridRowAlign(grid_row_align): val [GridAlign] [LAYOUT] GRID_START ["LV_STYLE_GRID_ROW_ALIGN"];
            /// Cell column (`LV_STYLE_GRID_CELL_COLUMN_POS`).
            GridCellColumn(grid_cell_column): val [i32] [LAYOUT] INT0 ["grid_cell_column_pos" "LV_STYLE_GRID_CELL_COLUMN_POS"];
            /// Cell column span (`LV_STYLE_GRID_CELL_COLUMN_SPAN`).
            GridCellColumnSpan(grid_cell_column_span): val [i32] [LAYOUT] INT1 ["LV_STYLE_GRID_CELL_COLUMN_SPAN"];
            /// Horizontal alignment in the cell (`LV_STYLE_GRID_CELL_X_ALIGN`).
            GridCellXAlign(grid_cell_x_align): val [GridAlign] [LAYOUT] GRID_START ["LV_STYLE_GRID_CELL_X_ALIGN"];
            /// Cell row (`LV_STYLE_GRID_CELL_ROW_POS`).
            GridCellRow(grid_cell_row): val [i32] [LAYOUT] INT0 ["grid_cell_row_pos" "LV_STYLE_GRID_CELL_ROW_POS"];
            /// Cell row span (`LV_STYLE_GRID_CELL_ROW_SPAN`).
            GridCellRowSpan(grid_cell_row_span): val [i32] [LAYOUT] INT1 ["LV_STYLE_GRID_CELL_ROW_SPAN"];
            /// Vertical alignment in the cell (`LV_STYLE_GRID_CELL_Y_ALIGN`).
            GridCellYAlign(grid_cell_y_align): val [GridAlign] [LAYOUT] GRID_START ["LV_STYLE_GRID_CELL_Y_ALIGN"];
        }
    };
}

/// Hands the shorthand table to the callback macro `$cb` as `$cb! { [args] rows… }` (see the
/// module documentation of `table.rs` for the row format).
#[doc(hidden)]
#[macro_export]
macro_rules! __shorthand_table {
    ($cb:ident $($args:tt)*) => {
        $cb! {
            [$($args)*]
            /// Width and height.
            size(width: len<W> [Length] => Width; height: len<H> [Length] => Height) [] {
                /// ```
                /// # use twine_style::{Length, PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().size(100, Length::pct(50));
                /// assert_eq!(s.get(PropId::Width), Some(StyleValue::Length(Length::Px(100))));
                /// assert_eq!(s.get(PropId::Height), Some(StyleValue::Length(Length::Pct(50))));
                /// ```
            };
            /// X and Y position.
            pos(x: len<X> [Length] => X; y: len<Y> [Length] => Y) [] {
                /// ```
                /// # use twine_style::{Length, PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().pos(10, 20);
                /// assert_eq!(s.get(PropId::X), Some(StyleValue::Length(Length::Px(10))));
                /// assert_eq!(s.get(PropId::Y), Some(StyleValue::Length(Length::Px(20))));
                /// ```
            };
            /// Translation after layout (`translate_x`, `translate_y`).
            translate(x: len<X> [Length] => TranslateX; y: len<Y> [Length] => TranslateY) [] {
                /// ```
                /// # use twine_style::{Length, PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().translate(Length::pct(10), -4);
                /// assert_eq!(s.get(PropId::TranslateX), Some(StyleValue::Length(Length::Pct(10))));
                /// assert_eq!(s.get(PropId::TranslateY), Some(StyleValue::Length(Length::Px(-4))));
                /// ```
            };
            /// Translation after layout as one point in pixels (handy to bind an animated
            /// `Point`).
            offset(point: val [Point] => TranslateX(px .x), TranslateY(px .y)) [] {
                /// ```
                /// # use twine_core::Point;
                /// # use twine_style::{Length, PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().offset(Point::new(3, -2));
                /// assert_eq!(s.get(PropId::TranslateX), Some(StyleValue::Length(Length::Px(3))));
                /// assert_eq!(s.get(PropId::TranslateY), Some(StyleValue::Length(Length::Px(-2))));
                /// ```
            };
            /// Padding on all four sides.
            padding(v: len<V> [Length] => PaddingTop, PaddingBottom, PaddingLeft, PaddingRight) ["pad_all"] {
                /// ```
                /// # use twine_style::{Length, PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().padding(8);
                /// assert_eq!(s.len(), 4);
                /// assert_eq!(s.get(PropId::PaddingLeft), Some(StyleValue::Length(Length::Px(8))));
                /// // Density-independent: resolved with the display's DPI.
                /// let s = StyleBuf::new().padding(Length::dp(8));
                /// assert_eq!(s.get(PropId::PaddingTop), Some(StyleValue::Length(Length::Dp(8))));
                /// ```
            };
            /// Left and right padding.
            padding_x(v: len<V> [Length] => PaddingLeft, PaddingRight) ["pad_hor" "padding_hor"] {
                /// ```
                /// # use twine_style::{Length, PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().padding_x(6);
                /// assert_eq!(s.get(PropId::PaddingRight), Some(StyleValue::Length(Length::Px(6))));
                /// assert_eq!(s.get(PropId::PaddingTop), None);
                /// ```
            };
            /// Top and bottom padding.
            padding_y(v: len<V> [Length] => PaddingTop, PaddingBottom) ["pad_ver" "padding_ver"] {
                /// ```
                /// # use twine_style::{Length, PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().padding_y(2);
                /// assert_eq!(s.get(PropId::PaddingBottom), Some(StyleValue::Length(Length::Px(2))));
                /// assert_eq!(s.get(PropId::PaddingLeft), None);
                /// ```
            };
            /// Padding per side.
            padding_each(insets: val [Insets] => PaddingTop(px .top), PaddingBottom(px .bottom), PaddingLeft(px .left), PaddingRight(px .right)) [] {
                /// ```
                /// # use twine_core::Insets;
                /// # use twine_style::{Length, PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().padding_each(Insets::new(1, 2, 3, 4));
                /// assert_eq!(s.len(), 4);
                /// assert_eq!(s.get(PropId::PaddingTop), Some(StyleValue::Length(Length::Px(Insets::new(1, 2, 3, 4).top))));
                /// ```
            };
            /// Margin on all four sides.
            margin(v: len<V> [Length] => MarginTop, MarginBottom, MarginLeft, MarginRight) ["margin_all"] {
                /// ```
                /// # use twine_style::{Length, PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().margin(5);
                /// assert_eq!(s.get(PropId::MarginBottom), Some(StyleValue::Length(Length::Px(5))));
                /// ```
            };
            /// Left and right margin.
            margin_x(v: len<V> [Length] => MarginLeft, MarginRight) ["margin_hor"] {
                /// ```
                /// # use twine_style::{Length, PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().margin_x(5);
                /// assert_eq!(s.get(PropId::MarginLeft), Some(StyleValue::Length(Length::Px(5))));
                /// assert_eq!(s.get(PropId::MarginTop), None);
                /// ```
            };
            /// Top and bottom margin.
            margin_y(v: len<V> [Length] => MarginTop, MarginBottom) ["margin_ver"] {
                /// ```
                /// # use twine_style::{Length, PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().margin_y(5);
                /// assert_eq!(s.get(PropId::MarginTop), Some(StyleValue::Length(Length::Px(5))));
                /// assert_eq!(s.get(PropId::MarginLeft), None);
                /// ```
            };
            /// Margin per side.
            margin_each(insets: val [Insets] => MarginTop(px .top), MarginBottom(px .bottom), MarginLeft(px .left), MarginRight(px .right)) [] {
                /// ```
                /// # use twine_core::Insets;
                /// # use twine_style::{Length, PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().margin_each(Insets::new(1, 2, 3, 4));
                /// assert_eq!(s.len(), 4);
                /// assert_eq!(s.get(PropId::MarginRight), Some(StyleValue::Length(Length::Px(Insets::new(1, 2, 3, 4).right))));
                /// ```
            };
            /// Gap between the rows and between the columns of a flex or grid container.
            gap(v: len<V> [Length] => RowGap, ColumnGap) ["pad_gap"] {
                /// ```
                /// # use twine_style::{Length, PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().gap(4);
                /// assert_eq!(s.get(PropId::RowGap), Some(StyleValue::Length(Length::Px(4))));
                /// assert_eq!(s.get(PropId::ColumnGap), Some(StyleValue::Length(Length::Px(4))));
                /// ```
            };
            /// Background color, fully opaque (`bg_color` + `bg_opacity: Opa::COVER`).
            bg(color: val [Color] => BgColor, BgOpacity = OPA_COVER) [] {
                /// ```
                /// # use twine_core::{Color, Opa};
                /// # use twine_style::{PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().bg(Color::RED);
                /// assert_eq!(s.get(PropId::BgColor), Some(StyleValue::Color(Color::RED)));
                /// assert_eq!(s.get(PropId::BgOpacity), Some(StyleValue::Opa(Opa::COVER)));
                /// ```
            };
            /// Border width and color, fully opaque (`border_width`, `border_color`,
            /// `border_opacity: Opa::COVER`).
            border(width: len<W> [Length] => BorderWidth; color: val [Color] => BorderColor, BorderOpacity = OPA_COVER) [] {
                /// ```
                /// # use twine_core::{Color, Opa};
                /// # use twine_style::{Length, PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().border(2, Color::BLUE);
                /// assert_eq!(s.get(PropId::BorderWidth), Some(StyleValue::Length(Length::Px(2))));
                /// assert_eq!(s.get(PropId::BorderColor), Some(StyleValue::Color(Color::BLUE)));
                /// assert_eq!(s.get(PropId::BorderOpacity), Some(StyleValue::Opa(Opa::COVER)));
                /// ```
            };
            /// Outline width, color (fully opaque) and offset from the node.
            outline(width: val [i32] => OutlineWidth; color: val [Color] => OutlineColor, OutlineOpacity = OPA_COVER; offset: val [i32] => OutlineOffset) [] {
                /// ```
                /// # use twine_core::Color;
                /// # use twine_style::{PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().outline(2, Color::GREEN, 3);
                /// assert_eq!(s.len(), 4);
                /// assert_eq!(s.get(PropId::OutlineOffset), Some(StyleValue::Int(3)));
                /// ```
            };
            /// Every shadow property from one `ShadowDsc`.
            shadow(shadow: val [ShadowDsc] => ShadowWidth(.width), ShadowOffsetX(.ofs_x), ShadowOffsetY(.ofs_y), ShadowSpread(.spread), ShadowColor(.color), ShadowOpacity(.opa)) [] {
                /// ```
                /// # use twine_core::{Color, Opa};
                /// # use twine_render::ShadowDsc;
                /// # use twine_style::{PropId, StyleBuf, StyleValue};
                /// let d = ShadowDsc { width: 8, ofs_x: 0, ofs_y: 2, spread: 0, color: Color::BLACK, opa: Opa::P30 };
                /// let s = StyleBuf::new().shadow(d);
                /// assert_eq!(s.len(), 6);
                /// assert_eq!(s.get(PropId::ShadowOffsetY), Some(StyleValue::Int(2)));
                /// ```
            };
            /// Shadow offset.
            shadow_offset(x: val [i32] => ShadowOffsetX; y: val [i32] => ShadowOffsetY) [] {
                /// ```
                /// # use twine_style::{PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().shadow_offset(1, 3);
                /// assert_eq!(s.get(PropId::ShadowOffsetY), Some(StyleValue::Int(3)));
                /// ```
            };
            /// Scale of the rendered node on both axes.
            transform_scale(scale: val [Scale] => TransformScaleX, TransformScaleY) [] {
                /// ```
                /// # use twine_core::Scale;
                /// # use twine_style::{PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().transform_scale(Scale::pct(120));
                /// assert_eq!(s.get(PropId::TransformScaleY), Some(StyleValue::Scale(Scale::pct(120))));
                /// ```
            };
            /// Pivot of the transformation in pixels, relative to the node.
            transform_pivot(point: val [Point] => TransformPivotX(px .x), TransformPivotY(px .y)) [] {
                /// ```
                /// # use twine_core::Point;
                /// # use twine_style::{Length, PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().transform_pivot(Point::new(5, 6));
                /// assert_eq!(s.get(PropId::TransformPivotY), Some(StyleValue::Length(Length::Px(6))));
                /// ```
            };
            /// Grid columns of the item: a column (`2`) or a range of columns (`0..2`), see
            /// `GridSpan`.
            grid_col(columns: span [GridSpan] => GridCellColumn(.start), GridCellColumnSpan(.span)) ["grid_cell"] {
                /// ```
                /// # use twine_style::{PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().grid_col(1..3);
                /// assert_eq!(s.get(PropId::GridCellColumn), Some(StyleValue::Int(1)));
                /// assert_eq!(s.get(PropId::GridCellColumnSpan), Some(StyleValue::Int(2)));
                /// ```
            };
            /// Grid rows of the item: a row (`1`) or a range of rows (`0..=1`), see `GridSpan`.
            grid_row(rows: span [GridSpan] => GridCellRow(.start), GridCellRowSpan(.span)) ["grid_cell"] {
                /// ```
                /// # use twine_style::{PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().grid_row(2);
                /// assert_eq!(s.get(PropId::GridCellRow), Some(StyleValue::Int(2)));
                /// assert_eq!(s.get(PropId::GridCellRowSpan), Some(StyleValue::Int(1)));
                /// ```
            };
            /// Alignment inside the grid cell (horizontal, vertical).
            grid_align(x: val [GridAlign] => GridCellXAlign; y: val [GridAlign] => GridCellYAlign) ["grid_cell_align"] {
                /// ```
                /// # use twine_style::{GridAlign, PropId, StyleBuf, StyleValue};
                /// let s = StyleBuf::new().grid_align(GridAlign::Center, GridAlign::End);
                /// assert_eq!(s.get(PropId::GridCellXAlign), Some(StyleValue::from(GridAlign::Center)));
                /// ```
            };
        }
    };
}

/// Resolves a payload type of the tables (`i32`, `Length`, `&'static Gradient`,
/// `&'static [GridTrack]`, …) to its fully qualified path.
#[doc(hidden)]
#[macro_export]
macro_rules! __prop_ty {
    (i32) => { ::core::primitive::i32 };
    (u32) => { ::core::primitive::u32 };
    (u8) => { ::core::primitive::u8 };
    (u16) => { ::core::primitive::u16 };
    (bool) => { ::core::primitive::bool };
    (&'static [$t:ident]) => { &'static [$crate::__private::$t] };
    (&'static $t:ident) => { &'static $crate::__private::$t };
    ($t:ident) => { $crate::__private::$t };
}

/// The parameter type of a `StyleBuf` builder: `impl Into<payload>`, except `u16` (flex weights),
/// taken exactly so that an integer literal infers (`u8` also converts into `u16`).
#[doc(hidden)]
#[macro_export]
macro_rules! __builder_ty {
    (u16) => { ::core::primitive::u16 };
    ($($ty:tt)+) => { impl ::core::convert::Into<$crate::__prop_ty!($($ty)+)> };
}

/// The display name of a payload type of the tables (`"&'static Gradient"`).
#[doc(hidden)]
#[macro_export]
macro_rules! __prop_type_name {
    (&'static [$t:ident]) => {
        ::core::concat!("&'static [", ::core::stringify!($t), "]")
    };
    (&'static $t:ident) => {
        ::core::concat!("&'static ", ::core::stringify!($t))
    };
    ($t:ident) => {
        ::core::stringify!($t)
    };
}

/// One property of a shorthand as a `StyleProp` expression: `[Variant] [selector] [constant]
/// kind value` (see the shorthand row format in `table.rs`).
#[doc(hidden)]
#[macro_export]
macro_rules! __shorthand_prop {
    ([$var:ident] [] [] $kind:ident $v:expr) => {
        $crate::StyleProp::$var($crate::__style_wrap!($kind, $v))
    };
    ([$var:ident] [. $f:ident] [] $kind:ident $v:expr) => {
        $crate::StyleProp::$var($v.$f)
    };
    ([$var:ident] [px . $f:ident] [] $kind:ident $v:expr) => {
        $crate::StyleProp::$var($crate::Length::Px($v.$f))
    };
    ([$var:ident] [] [$c:ident] $kind:ident $v:expr) => {
        $crate::StyleProp::$var($crate::__private::$c)
    };
}
