// Unit-typed properties take typed values only: a bare integer is not an opacity, an angle
// or a scale (write `Opa::pct(50)`, `Angle::deg(30)`, `Scale::pct(98)`).
use twine_style::{Style, style};

static OPACITY: Style = style! { bg_opacity: 128 };
static ROTATION: Style = style! { transform_rotation: 300 };
static SCALE: Style = style! { transform_scale_x: 256 };
static SCALE_BOTH: Style = style! { transform_scale: 250 };

fn main() {
    let _ = (&OPACITY, &ROTATION, &SCALE, &SCALE_BOTH);
}
