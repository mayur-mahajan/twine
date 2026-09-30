use twine_style::{Style, style};

// The former LVGL-style names are documentation aliases only, not keys.
static S: Style = style! { pad_all: 4 };

fn main() {
    let _ = &S;
}
