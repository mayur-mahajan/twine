//! `cargo xtask sim navigation`: typed navigation. The routes are an enum; the home page has
//! one button per `ScreenAnim` kind (each pushes the settings page with that animation) and a
//! modal; the settings page has "Back" and an editor with unsaved changes, whose leave guard
//! asks before discarding them.

use twine::prelude::*;
use twine_sim::SimConfig;

const D: Duration = Duration::ms(300);

/// The pages of the app.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Route {
    Home,
    Settings,
    Editor,
}

fn app(cx: Scope) -> impl View {
    navigator(cx, Route::Home, |cx, route| match route {
        Route::Home => home(cx).into_any(),
        Route::Settings => settings(cx).into_any(),
        Route::Editor => editor(cx).into_any(),
    })
}

fn anim_button(nav: Navigator<Route>, anim: ScreenAnim) -> impl View {
    button(label(anim.name())).on_click(move || nav.push(Route::Settings, anim))
}

fn home(cx: Scope) -> impl View {
    let nav = expect_navigator::<Route>(cx);
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
    let nav = expect_navigator::<Route>(cx);
    column((
        label("Settings").font(&fonts::MONTSERRAT_20),
        label(text!("Depth {}", nav.depth())),
        button(label("Editor")).on_click(move || nav.push(Route::Editor, ScreenAnim::OverLeft(D))),
        button(label("Back")).on_click(move || nav.pop(ScreenAnim::MoveRight(D))),
    ))
    .gap(12)
    .padding(16)
    .align_items(CrossAlign::Center)
    .size(Length::pct(100), Length::pct(100))
}

/// A page with unsaved changes: leaving it asks first (a leave guard).
fn editor(cx: Scope) -> impl View {
    let nav = expect_navigator::<Route>(cx);
    let text = cx.signal(String::new());
    nav.on_leave(cx, move |_to: &Route| {
        if text.with(String::is_empty) {
            return Leave::Allow;
        }
        let _ = cx.show_modal(move |_cx, modal: ModalHandle| {
            container(
                column((
                    label("Discard your changes?"),
                    row((
                        button(label("Discard")).on_click(move || {
                            text.set(String::new());
                            modal.close();
                            nav.pop(ScreenAnim::MoveRight(D));
                        }),
                        button(label("Keep editing")).on_click(move || modal.close()),
                    ))
                    .gap(8),
                ))
                .gap(8)
                .align_items(CrossAlign::Center),
            )
        });
        Leave::Deny
    });
    column((
        label("Editor").font(&fonts::MONTSERRAT_20),
        textarea(text)
            .placeholder("Type, then go back")
            .width(Length::pct(100)),
        button(label("Back")).on_click(move || nav.pop(ScreenAnim::MoveRight(D))),
    ))
    .gap(8)
    .padding(16)
    .size(Length::pct(100), Length::pct(100))
}

fn main() {
    twine_sim::run(
        SimConfig::new(320, 240)
            .title("navigation")
            .scale(2)
            .theme(&lvgl::THEME),
        app,
    );
}
