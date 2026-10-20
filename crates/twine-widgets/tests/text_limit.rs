//! R5.S03: `Limits::max_text_len` cuts widget texts (labels, checkboxes, spans, button-matrix
//! buttons) at a character boundary with a `Capacity` fault (`TextLen`), and with
//! `Limits::preallocate` their buffers are sized once, so changing a text after a seal
//! allocates nothing (this binary installs the counting allocator).
//!
//! req: SAF-008

mod common;

use core::fmt::Write as _;

use common::{Mode, harness, with};
use twine_core::Limits;
use twine_core::fault::{CapacityFault, FaultKind};
use twine_testing::alloc::{CountingAllocator, assert_no_alloc_after_seal};
use twine_widgets::buttonmatrix::{self, BtnCtrl, BtnMap, ButtonMatrix, OwnedMap};
use twine_widgets::checkbox::{self, Checkbox};
use twine_widgets::label::{self, Label};
use twine_widgets::spangroup::{self, SpanGroup};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

#[test]
fn texts_are_cut_to_max_text_len_with_a_fault() {
    let mut h = harness(200, 60, Mode::Light);
    h.engine_mut()
        .set_limits(&Limits::DEFAULT.max_text_len(5))
        .unwrap();
    let screen = h.screen();
    let l = label::create(h.engine_mut(), screen).unwrap();
    with(&mut h, l, |w: &mut Label, cx| w.set_text(cx, "abcdéfgh"));
    assert_eq!(
        h.engine().widget::<Label>(l).unwrap().text(),
        "abcd",
        "cut before `é`"
    );
    let r = *h.engine().last_fault(FaultKind::Capacity).unwrap();
    assert_eq!(CapacityFault::from_code(r.code), Some(CapacityFault::TextLen));
    assert_eq!(r.node, Some(l));
    with(&mut h, l, |w: &mut Label, cx| {
        w.write_text(cx, |s| {
            let _ = write!(s, "{}", 1_234_567);
        })
    });
    assert_eq!(h.engine().widget::<Label>(l).unwrap().text(), "12345");
    with(&mut h, l, |w: &mut Label, cx| {
        w.edit_text(cx, |s| s.push_str("xyz"));
    });
    assert_eq!(h.engine().widget::<Label>(l).unwrap().text(), "12345");
    let c = checkbox::create(h.engine_mut(), screen).unwrap();
    with(&mut h, c, |w: &mut Checkbox, cx| w.set_text(cx, "long label"));
    assert_eq!(h.engine().widget::<Checkbox>(c).unwrap().text(), "long ");
    assert_eq!(h.engine().fault_counts().get(FaultKind::Capacity), 4);
}

#[test]
fn preallocated_label_changes_text_after_seal_without_allocating() {
    let mut h = harness(200, 60, Mode::Light);
    h.engine_mut()
        .set_limits(&Limits::DEFAULT.max_text_len(32).preallocated())
        .unwrap();
    let screen = h.screen();
    let l = label::create(h.engine_mut(), screen).unwrap();
    // Start-up: a short text sizes the buffers for the limit.
    with(&mut h, l, |w: &mut Label, cx| w.set_text(cx, "0"));
    with(&mut h, l, |w: &mut Label, cx| {
        w.write_text(cx, |s| {
            let _ = write!(s, "{}", 1);
        })
    });
    h.run_until_idle();
    assert_no_alloc_after_seal(|| {
        for i in 0..20u32 {
            with(&mut h, l, |w: &mut Label, cx| {
                w.write_text(cx, |s| {
                    let _ = write!(s, "reading {} of {}", i * 1000, 1 << 20);
                })
            });
        }
        with(&mut h, l, |w: &mut Label, cx| {
            w.set_text(cx, "a text of 32 bytes, exactly....");
        });
    });
}

#[test]
fn span_and_button_texts_are_cut_to_max_text_len_with_a_fault() {
    let mut h = harness(200, 80, Mode::Light);
    h.engine_mut()
        .set_limits(&Limits::DEFAULT.max_text_len(4))
        .unwrap();
    let screen = h.screen();
    let g = spangroup::create(h.engine_mut(), screen).unwrap();
    let id = with(&mut h, g, |w: &mut SpanGroup, cx| w.add_span(cx));
    with(&mut h, g, |w: &mut SpanGroup, cx| {
        w.set_span_text(cx, id, "spanné");
    });
    let text = h
        .engine()
        .widget::<SpanGroup>(g)
        .unwrap()
        .span(id)
        .unwrap()
        .text()
        .to_string();
    assert_eq!(text, "span");
    let m = buttonmatrix::create(h.engine_mut(), screen).unwrap();
    with(&mut h, m, |w: &mut ButtonMatrix, cx| {
        w.set_btn_text(cx, 0, "Cancel");
    });
    assert_eq!(
        h.engine().widget::<ButtonMatrix>(m).unwrap().btn_text(0),
        Some("Canc")
    );
    // An owned map's texts are cut when it is set.
    let mut map = OwnedMap::new();
    map.push("OK", BtnCtrl::empty());
    map.push("Retry", BtnCtrl::empty());
    with(&mut h, m, |w: &mut ButtonMatrix, cx| {
        w.set_map(cx, BtnMap::Owned(map));
    });
    let w = h.engine().widget::<ButtonMatrix>(m).unwrap();
    assert_eq!((w.btn_text(0), w.btn_text(1)), (Some("OK"), Some("Retr")));
    let r = *h.engine().last_fault(FaultKind::Capacity).unwrap();
    assert_eq!(CapacityFault::from_code(r.code), Some(CapacityFault::TextLen));
    assert_eq!(r.node, Some(m));
    assert_eq!(h.engine().fault_counts().get(FaultKind::Capacity), 3);
}

#[test]
fn preallocated_span_and_button_texts_change_after_seal_without_allocating() {
    let mut h = harness(200, 80, Mode::Light);
    h.engine_mut()
        .set_limits(&Limits::DEFAULT.max_text_len(24).preallocated())
        .unwrap();
    let screen = h.screen();
    let g = spangroup::create(h.engine_mut(), screen).unwrap();
    let id = with(&mut h, g, |w: &mut SpanGroup, cx| w.add_span(cx));
    with(&mut h, g, |w: &mut SpanGroup, cx| w.set_span_text(cx, id, "0"));
    let m = buttonmatrix::create(h.engine_mut(), screen).unwrap();
    with(&mut h, m, |w: &mut ButtonMatrix, cx| w.set_btn_text(cx, 1, "0"));
    h.run_until_idle();
    let mut buf = String::with_capacity(24);
    assert_no_alloc_after_seal(|| {
        for i in 0..20u32 {
            buf.clear();
            let _ = write!(buf, "value {}", i * 99_991);
            with(&mut h, g, |w: &mut SpanGroup, cx| w.set_span_text(cx, id, &buf));
            with(&mut h, m, |w: &mut ButtonMatrix, cx| w.set_btn_text(cx, 1, &buf));
        }
    });
    assert_eq!(
        h.engine().widget::<ButtonMatrix>(m).unwrap().btn_text(1),
        Some("value 1899829")
    );
}
