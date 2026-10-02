//! Typed symbols and icons (R1.S08): `Symbol` as an image source and as text, and
//! `Icon` (`()` = no icon) for optional images, as a constant, closure or signal property.

use twine_engine::{NodeId, ObjFlags};
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{TestUi, by_id};
use twine_view::prelude::*;
use twine_widgets::image::Image;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

fn node(t: &TestUi, id: &'static str) -> NodeId {
    t.find(by_id(id)).id()
}

fn text_of(t: &TestUi, n: NodeId) -> String {
    t.engine()
        .widget::<Label>(n)
        .map(|l| l.text().to_string())
        .unwrap_or_default()
}

/// The image children of `n`.
fn images(t: &TestUi, n: NodeId) -> Vec<NodeId> {
    t.engine()
        .tree()
        .children(n)
        .filter(|&c| t.engine().widget::<Image>(c).is_some())
        .collect()
}

/// The icon property of `i`, as the optional image source it shows.
fn icon<M>(i: impl IntoProp<Icon, M>) -> Prop<Option<ImageSource>> {
    i.into_prop().map(|i| i.0)
}

#[test]
fn from_symbol_for_image_source() {
    static S: ImageSource = ImageSource::symbol(Symbol::Save);
    assert_eq!(ImageSource::from(Symbol::Ok), ImageSource::Symbol("\u{F00C}"));
    assert_eq!(ImageSource::symbol(Symbol::Close), Symbol::Close.into());
    assert_eq!(S, ImageSource::Symbol(Symbol::Save.as_str()));
}

#[test]
fn icon_constants_are_static() {
    assert!(matches!(icon(()), Prop::Static(None)));
    assert!(matches!(
        icon(Symbol::Ok),
        Prop::Static(Some(ImageSource::Symbol("\u{F00C}")))
    ));
    assert!(matches!(
        icon(ImageSource::symbol(Symbol::Save)),
        Prop::Static(Some(ImageSource::Symbol("\u{F0C7}")))
    ));
    assert!(matches!(icon(Some(Symbol::Up)), Prop::Static(Some(_))));
    assert!(matches!(icon(None::<Symbol>), Prop::Static(None)));
    assert!(matches!(
        icon(Some(ImageSource::symbol(Symbol::Up))),
        Prop::Static(Some(_))
    ));
    assert!(matches!(icon(None::<ImageSource>), Prop::Static(None)));
}

#[test]
fn icon_reactive_forms_are_dynamic() {
    let cx = twine_reactive::create_root();
    let src: Signal<Option<ImageSource>> = cx.signal(None);
    let sym = cx.signal(Symbol::Ok);
    let opt = cx.signal(Some(Symbol::Ok));
    let shown = cx.signal(true);
    assert!(matches!(icon(src), Prop::Dynamic(_)));
    assert!(matches!(icon(sym), Prop::Dynamic(_)));
    assert!(matches!(icon(opt), Prop::Dynamic(_)));
    let Prop::Dynamic(f) = icon(move || shown.get().then_some(Symbol::Wifi)) else {
        panic!("closure icon must be dynamic");
    };
    assert_eq!(f(), Some(ImageSource::symbol(Symbol::Wifi)));
    shown.set(false);
    assert_eq!(f(), None);
    // Closures returning a bare symbol or image source, and signals of image sources, are
    // icons too (every type converting into `Icon`).
    assert!(matches!(icon(move || Symbol::Ok), Prop::Dynamic(_)));
    assert!(matches!(
        icon(move || ImageSource::symbol(Symbol::Ok)),
        Prop::Dynamic(_)
    ));
    assert!(matches!(
        icon(cx.signal(ImageSource::symbol(Symbol::Ok))),
        Prop::Dynamic(_)
    ));
    assert!(matches!(
        icon(cx.memo(move || shown.get().then_some(Symbol::Ok))),
        Prop::Dynamic(_)
    ));
    cx.dispose();
}

#[test]
fn symbol_is_an_image_prop_and_a_text() {
    fn src<M>(p: impl IntoProp<ImageSource, M>) -> Prop<ImageSource> {
        p.into_prop()
    }
    assert!(matches!(
        src(Symbol::Play),
        Prop::Static(ImageSource::Symbol("\u{F04B}"))
    ));
    assert!(matches!(
        Symbol::Play.into_text(),
        twine_view::TextProp::Static("\u{F04B}")
    ));
}

#[test]
fn list_button_icons() {
    let t = TestUi::new(240, 200).mount(|_| {
        list((
            list_button((), "plain").test_id("none"),
            list_button(Symbol::File, "file").test_id("sym"),
            list_button(ImageSource::symbol(Symbol::Save), "save").test_id("src"),
            list_button(Some(Symbol::Trash), "trash").test_id("opt"),
        ))
    });
    // `()` creates no image at all (no node, no binding).
    assert!(images(&t, node(&t, "none")).is_empty());
    for (id, s) in [
        ("sym", Symbol::File),
        ("src", Symbol::Save),
        ("opt", Symbol::Trash),
    ] {
        let imgs = images(&t, node(&t, id));
        assert_eq!(imgs.len(), 1, "{id}");
        assert_eq!(
            t.engine().widget::<Image>(imgs[0]).unwrap().src(),
            Some(&ImageSource::symbol(s)),
            "{id}"
        );
    }
}

#[test]
fn reactive_symbol_icon_follows_signal() {
    let mut t = TestUi::new(240, 200).mount(|cx| {
        let s: Signal<Option<Symbol>> = cx.signal(Some(Symbol::Play));
        cx.provide(s);
        list(list_button(s, "media").test_id("b"))
    });
    t.run_until_idle();
    let s = t.root_scope().expect_context::<Signal<Option<Symbol>>>();
    let img = images(&t, node(&t, "b"))[0];
    s.set(Some(Symbol::Pause));
    t.run_until_idle();
    assert_eq!(
        t.engine().widget::<Image>(img).unwrap().src(),
        Some(&ImageSource::symbol(Symbol::Pause))
    );
    assert!(!t.engine().has_flag(img, ObjFlags::HIDDEN));
    s.set(None);
    t.run_until_idle();
    assert!(t.engine().has_flag(img, ObjFlags::HIDDEN));
}

#[test]
fn dropdown_symbol_unit_removes_it() {
    let t = TestUi::new(240, 200).mount(|_| {
        column((
            dropdown(["a", "b"], 0usize).test_id("default"),
            dropdown(["a", "b"], 0usize).symbol(()).test_id("none"),
            dropdown(["a", "b"], 0usize).symbol(Symbol::Plus).test_id("plus"),
        ))
    });
    let sym = |id| {
        t.engine()
            .widget::<Dropdown>(node(&t, id))
            .unwrap()
            .symbol()
            .cloned()
    };
    assert_eq!(sym("default"), Some(ImageSource::symbol(Symbol::Down)));
    assert_eq!(sym("none"), None);
    assert_eq!(sym("plus"), Some(ImageSource::symbol(Symbol::Plus)));
}

#[test]
fn symbol_in_labels_and_text_macro() {
    let mut t = TestUi::new(240, 200).mount(|cx| {
        let n = cx.signal(3u32);
        cx.provide(n);
        column((
            label(Symbol::Ok).test_id("plain"),
            label(text!("{} Save", Symbol::Save)).test_id("static"),
            label(text!("{} {}", Symbol::Bell, n.get())).test_id("dynamic"),
            image(Symbol::Wifi).test_id("img"),
            window_button(Symbol::Close, 30).test_id("win"),
        ))
    });
    assert_eq!(text_of(&t, node(&t, "plain")), "\u{F00C}");
    assert_eq!(text_of(&t, node(&t, "static")), "\u{F0C7} Save");
    assert_eq!(text_of(&t, node(&t, "dynamic")), "\u{F0F3} 3");
    let n = t.root_scope().expect_context::<Signal<u32>>();
    n.set(4);
    t.run_until_idle();
    assert_eq!(text_of(&t, node(&t, "dynamic")), "\u{F0F3} 4");
    assert_eq!(
        t.engine().widget::<Image>(node(&t, "img")).unwrap().src(),
        Some(&ImageSource::symbol(Symbol::Wifi))
    );
    assert_eq!(images(&t, node(&t, "win")).len(), 1);
}

#[test]
fn symbol_display_does_not_allocate() {
    use core::fmt::Write;
    let mut buf = heapless_buf::Buf::default();
    let (r, stats) = count_allocs(|| write!(buf, "{} {}", Symbol::Left, Symbol::Right));
    r.unwrap();
    assert_eq!(stats.allocs, 0);
    assert_eq!(buf.as_str(), "\u{F053} \u{F054}");
}

mod heapless_buf {
    /// A fixed-size string buffer (no allocation).
    #[derive(Default)]
    pub struct Buf {
        bytes: [u8; 32],
        len: usize,
    }

    impl Buf {
        pub fn as_str(&self) -> &str {
            core::str::from_utf8(&self.bytes[..self.len]).unwrap()
        }
    }

    impl core::fmt::Write for Buf {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let end = self.len + s.len();
            self.bytes
                .get_mut(self.len..end)
                .ok_or(core::fmt::Error)?
                .copy_from_slice(s.as_bytes());
            self.len = end;
            Ok(())
        }
    }
}
