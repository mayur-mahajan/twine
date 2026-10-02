//! `cargo xtask sim navigation`: a navigator with every screen-load animation (one button per
//! `ScreenAnim` kind on the home screen), a settings screen with "Back", and a modal.

use twine::prelude::*;
use twine_sim::SimConfig;

const D: Duration = Duration::ms(300);

fn app(cx: Scope) -> impl View {
    navigator(cx, home)
}

fn anim_button(nav: Navigator, anim: ScreenAnim) -> impl View {
    button(label(anim.name())).on_click(move || nav.push(settings, anim))
}

fn home(cx: Scope) -> impl View {
    let nav = use_navigator(cx);
    let buttons: Vec<_> = ScreenAnim::all(D)
        .into_iter()
        .map(|a| anim_button(nav, a))
        .collect();
    column((
        label("Home").font(&fonts::MONTSERRAT_20),
        button(label("Open modal")).on_click(move || {
            let _ = cx.show_modal(|_cx, modal: ModalHandle| {
                container(
                    column((
                        label("A modal on the top layer"),
                        button(label("Close")).on_click(move || modal.close()),
                    ))
                    .gap(8)
                    .align_items(CrossAlign::Center),
                )
            });
        }),
        flex(FlexDirection::Row, buttons)
            .wrap(true)
            .gap(4)
            .width(Length::pct(100)),
    ))
    .gap(8)
    .padding(8)
    .size(Length::pct(100), Length::pct(100))
    .scrollable(true)
}

fn settings(cx: Scope) -> impl View {
    let nav = use_navigator(cx);
    column((
        label("Settings").font(&fonts::MONTSERRAT_20),
        button(label("Back")).on_click(move || nav.pop(ScreenAnim::MoveRight(D))),
    ))
    .gap(12)
    .padding(16)
    .align_items(CrossAlign::Center)
    .size(Length::pct(100), Length::pct(100))
}

fn main() {
    twine_sim::run(
        SimConfig::new(320, 240)
            .title("navigation")
            .scale(2)
            .theme(DefaultTheme::light()),
        app,
    );
}
