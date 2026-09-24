//! `Textarea`: the editing model (UTF-8 cursor, constraints, the insert filter, events) and
//! the interactive parts (blinking cursor only while focused, click positioning, selection,
//! password mode, scrolling to the cursor, placeholder, the default theme's look).

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use common::{Mode, get, harness, with};
use twine_core::{Duration, Point, Rect};
use twine_engine::{EventCode, EventFilter, EventResult, Key, MeasureCx, NodeId, State};
use twine_style::{Align, Part, PropId};
use twine_testing::EngineHarness;
use twine_widgets::label::Label;
use twine_widgets::textarea::{self, AcceptedChars, InsertCx, TEXTAREA_CLASS, Textarea};

/// A 200 × 100 textarea at the top of a 240 × 160 screen, in the default group.
fn scene(mode: Mode) -> (EngineHarness, NodeId) {
    let mut h = harness(240, 160, mode);
    let g = h.engine_mut().create_group().unwrap();
    h.engine_mut().set_default_group(Some(g));
    let screen = h.screen();
    let e = h.engine_mut();
    // A button first: it gets the focus of the group, the textarea starts unfocused.
    let b = twine_widgets::button::create(e, screen).unwrap();
    e.set_size(b, 10, 10);
    e.align(b, Align::BottomRight, 0, 0);
    let ta = textarea::create(e, screen).unwrap();
    e.set_size(ta, 200, 100);
    e.align(ta, Align::TopMid, 0, 10);
    h.run_until_idle();
    (h, ta)
}

fn text(h: &EngineHarness, ta: NodeId) -> String {
    textarea::text_of(h.engine(), ta).unwrap().to_owned()
}

fn cursor(h: &EngineHarness, ta: NodeId) -> usize {
    get::<Textarea>(h, ta).cursor_pos()
}

fn value_changes(h: &mut EngineHarness, ta: NodeId) -> Rc<RefCell<Vec<String>>> {
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    h.engine_mut()
        .add_event_handler(ta, EventFilter::Code(EventCode::ValueChanged), move |cx, _| {
            // The event arrives once the textarea is back in its node: read the widget.
            let e = cx.engine();
            let ta = e.widget::<Textarea>(cx.target()).expect("textarea readable");
            let t = ta.text(&MeasureCx::new(e, cx.target())).to_owned();
            l.borrow_mut().push(t);
            EventResult::Continue
        });
    log
}

mod textarea_model {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn add_char_at_cursor_utf8() {
        let (mut h, ta) = scene(Mode::Light);
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.add_text(cx, "ab");
            t.set_cursor_pos(cx, 1);
            t.add_char(cx, 'ä');
            t.add_char(cx, '你');
            t.add_char(cx, '😀');
        });
        assert_eq!(text(&h, ta), "aä你😀b");
        assert_eq!(cursor(&h, ta), 4, "cursor positions are characters");
        let c = get::<Textarea>(&h, ta).cursor();
        assert_eq!(c.byte, "aä你😀".len());
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.cursor_left(cx);
            t.add_char(cx, 'x');
        });
        assert_eq!(text(&h, ta), "aä你x😀b");
        // Negative positions count from the end, CURSOR_LAST is the end.
        with(&mut h, ta, |t: &mut Textarea, cx| t.set_cursor_pos(cx, -1));
        assert_eq!(cursor(&h, ta), 5);
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.set_cursor_pos(cx, textarea::CURSOR_LAST);
        });
        assert_eq!(cursor(&h, ta), 6);
        let current = get::<Textarea>(&h, ta).current_char(&MeasureCx::new(h.engine(), ta));
        assert_eq!(current, Some('b'));
    }

    #[test]
    fn delete_char_backspace_and_forward() {
        let (mut h, ta) = scene(Mode::Light);
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.add_text(cx, "hé你o");
            t.set_cursor_pos(cx, 2);
            t.delete_char(cx);
        });
        assert_eq!(text(&h, ta), "h你o");
        assert_eq!(cursor(&h, ta), 1);
        with(&mut h, ta, |t: &mut Textarea, cx| t.delete_char_forward(cx));
        assert_eq!(text(&h, ta), "ho");
        assert_eq!(cursor(&h, ta), 1);
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.set_cursor_pos(cx, 0);
            t.delete_char(cx); // nothing before the cursor
            t.set_cursor_pos(cx, textarea::CURSOR_LAST);
            t.delete_char_forward(cx); // nothing after it
        });
        assert_eq!(text(&h, ta), "ho");
        // The keys do the same.
        let _ = h.keypad_input();
        h.engine_mut().focus(ta);
        h.key(Key::Backspace);
        assert_eq!(text(&h, ta), "h");
        h.key(Key::Home);
        h.key(Key::Del);
        assert_eq!(text(&h, ta), "");
    }

    #[test]
    fn max_length_enforced() {
        let (mut h, ta) = scene(Mode::Light);
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.set_max_length(cx, 4);
            t.add_text(cx, "ab你cdef");
            t.add_char(cx, 'x');
        });
        assert_eq!(text(&h, ta), "ab你c", "4 characters, not bytes");
        assert_eq!(cursor(&h, ta), 4);
        with(&mut h, ta, |t: &mut Textarea, cx| t.set_text(cx, "123456"));
        assert_eq!(text(&h, ta), "1234");
    }

    #[test]
    fn accepted_chars_filter() {
        let (mut h, ta) = scene(Mode::Light);
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.set_accepted_chars(cx, Some(AcceptedChars::Static("0123456789.")));
            t.add_text(cx, "3a.1-4");
            t.add_char(cx, 'z');
            t.add_char(cx, '5');
        });
        assert_eq!(text(&h, ta), "3.145");
        // Keys go through the same filter.
        let _ = h.keypad_input();
        h.engine_mut().focus(ta);
        h.type_text("x9");
        assert_eq!(text(&h, ta), "3.1459");
    }

    #[test]
    fn one_line_ignores_newline() {
        let (mut h, ta) = scene(Mode::Light);
        let ready = Rc::new(Cell::new(0));
        let r = ready.clone();
        h.engine_mut()
            .add_event_handler(ta, EventFilter::Code(EventCode::Ready), move |_, _| {
                r.set(r.get() + 1);
                EventResult::Continue
            });
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.set_one_line(cx, true);
            t.add_text(cx, "a\nb");
            t.add_char(cx, '\n');
            t.add_char(cx, '\r');
        });
        assert_eq!(text(&h, ta), "ab");
        let _ = h.keypad_input();
        h.engine_mut().focus(ta);
        h.key(Key::Enter);
        assert_eq!(text(&h, ta), "ab");
        assert_eq!(ready.get(), 1, "Enter sends Ready in one-line mode");
        // One line high: the height follows the text.
        h.update();
        let font_h = i32::from(twine_assets::fonts::MONTSERRAT_14.line_height);
        let c = h.engine().content_area(ta);
        assert_eq!(c.height(), font_h);
    }

    #[test]
    fn insert_event_can_replace_or_cancel() {
        let (mut h, ta) = scene(Mode::Light);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let s = seen.clone();
        h.engine_mut()
            .add_event_handler(ta, EventFilter::Code(EventCode::Insert), move |cx, ev| {
                s.borrow_mut().push(cx.text(ev).map(str::to_owned));
                EventResult::Continue
            });
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.set_insert_filter(Some(Box::new(|ins: &mut InsertCx<'_>| match ins.text() {
                "x" => ins.cancel(),
                "k" => ins.replace("K!"),
                t if t == textarea::DELETE_TEXT => ins.cancel(),
                _ => {}
            })));
            t.add_char(cx, 'a');
            t.add_char(cx, 'x');
            t.add_char(cx, 'k');
            t.delete_char(cx);
        });
        assert_eq!(text(&h, ta), "aK!", "x dropped, k replaced, deletion cancelled");
        assert_eq!(cursor(&h, ta), 3);
        // `Insert` carries the (non-static) text; the replacement is announced too.
        let seen = seen.borrow();
        let s = |t: &str| Some(t.to_owned());
        assert_eq!(*seen, [s("a"), s("x"), s("k"), s("K!"), s(textarea::DELETE_TEXT)]);
    }

    #[test]
    fn cursor_up_down_keeps_x() {
        let (mut h, ta) = scene(Mode::Light);
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.add_text(cx, "abcdefgh\nab\nabcdefgh");
            t.set_cursor_pos(cx, 6); // line 0, after "abcdef"
        });
        h.run_until_idle();
        let x0 = get::<Textarea>(&h, ta).cursor().valid_x;
        with(&mut h, ta, |t: &mut Textarea, cx| t.cursor_down(cx));
        assert_eq!(cursor(&h, ta), 11, "the end of the short line");
        with(&mut h, ta, |t: &mut Textarea, cx| t.cursor_down(cx));
        assert_eq!(cursor(&h, ta), 18, "back to x of the first line");
        assert_eq!(get::<Textarea>(&h, ta).cursor().valid_x, x0);
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.cursor_up(cx);
            t.cursor_up(cx);
        });
        assert_eq!(cursor(&h, ta), 6);
        // On the last line, down does nothing.
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.set_cursor_pos(cx, textarea::CURSOR_LAST);
            t.cursor_down(cx);
        });
        assert_eq!(cursor(&h, ta), 20);
    }

    #[test]
    fn set_text_same_no_invalidate() {
        let (mut h, ta) = scene(Mode::Light);
        with(&mut h, ta, |t: &mut Textarea, cx| t.set_text(cx, "same"));
        h.run_until_idle();
        let log = value_changes(&mut h, ta);
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.set_text(cx, "same");
            t.set_cursor_pos(cx, 4);
            t.set_max_length(cx, 0);
            t.set_one_line(cx, false);
            t.set_password_mode(cx, false);
            t.set_placeholder_text(cx, "");
            t.set_accepted_chars(cx, None);
            t.set_text_selection(cx, false);
            t.set_cursor_click_pos(cx, true);
        });
        assert!(h.engine().invalidation_log().is_empty());
        assert!(log.borrow().is_empty());
        h.assert_idle();
    }

    #[test]
    fn value_changed_per_edit() {
        let (mut h, ta) = scene(Mode::Light);
        let log = value_changes(&mut h, ta);
        with(&mut h, ta, |t: &mut Textarea, cx| t.add_char(cx, 'a'));
        with(&mut h, ta, |t: &mut Textarea, cx| t.add_text(cx, "bc"));
        with(&mut h, ta, |t: &mut Textarea, cx| t.delete_char(cx));
        with(&mut h, ta, |t: &mut Textarea, cx| t.set_text(cx, "xyz"));
        with(&mut h, ta, |t: &mut Textarea, cx| t.cursor_left(cx)); // not an edit
        assert_eq!(*log.borrow(), vec!["a", "abc", "ab", "xyz"]);
        // Several edits in one setter call: one event each, dispatched once the textarea is
        // back in its node (the handlers see the final text).
        log.borrow_mut().clear();
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.add_char(cx, '1');
            t.add_char(cx, '2');
        });
        assert_eq!(*log.borrow(), vec!["xy12z", "xy12z"]);
    }

    #[derive(Clone, Debug)]
    enum Op {
        Char(char),
        Text(String),
        Backspace,
        Del,
        Left,
        Right,
        Pos(i32),
    }

    fn op() -> impl Strategy<Value = Op> {
        let ch = prop::sample::select(vec!['a', 'Z', ' ', 'ä', '你', '😀', '\n']);
        prop_oneof![
            4 => ch.clone().prop_map(Op::Char),
            1 => prop::collection::vec(ch, 0..4).prop_map(|v| Op::Text(v.into_iter().collect())),
            2 => Just(Op::Backspace),
            1 => Just(Op::Del),
            2 => Just(Op::Left),
            2 => Just(Op::Right),
            1 => (-6i32..12).prop_map(Op::Pos),
        ]
    }

    thread_local! {
        static H: RefCell<Option<(EngineHarness, NodeId)>> = const { RefCell::new(None) };
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(10_000))]
        #[test]
        fn random_edits_match_string_model(ops in prop::collection::vec(op(), 0..24)) {
            H.with(|cell| {
                let mut slot = cell.borrow_mut();
                let (h, ta) = slot.get_or_insert_with(|| {
                    let mut h = EngineHarness::new(200, 100);
                    let screen = h.screen();
                    let ta = textarea::create(h.engine_mut(), screen).unwrap();
                    (h, ta)
                });
                let ta = *ta;
                with(h, ta, |t: &mut Textarea, cx| {
                    t.set_text(cx, "");
                    t.set_cursor_pos(cx, 0);
                });
                let mut model: Vec<char> = Vec::new();
                let mut cur = 0usize;
                for op in &ops {
                    with(h, ta, |t: &mut Textarea, cx| match op {
                        Op::Char(c) => t.add_char(cx, *c),
                        Op::Text(s) => t.add_text(cx, s),
                        Op::Backspace => t.delete_char(cx),
                        Op::Del => t.delete_char_forward(cx),
                        Op::Left => t.cursor_left(cx),
                        Op::Right => t.cursor_right(cx),
                        Op::Pos(p) => t.set_cursor_pos(cx, *p),
                    });
                    match op {
                        Op::Char(c) => {
                            model.insert(cur, *c);
                            cur += 1;
                        }
                        Op::Text(s) => {
                            for c in s.chars() {
                                model.insert(cur, c);
                                cur += 1;
                            }
                        }
                        Op::Backspace => {
                            if cur > 0 {
                                cur -= 1;
                                model.remove(cur);
                            }
                        }
                        Op::Del => {
                            if cur < model.len() {
                                model.remove(cur);
                            }
                        }
                        Op::Left => cur = cur.saturating_sub(1),
                        Op::Right => cur = (cur + 1).min(model.len()),
                        Op::Pos(p) => {
                            cur = if *p < 0 {
                                (model.len() as i64 + i64::from(*p)).max(0) as usize
                            } else {
                                (*p as usize).min(model.len())
                            };
                        }
                    }
                    let expected: String = model.iter().collect();
                    prop_assert_eq!(textarea::text_of(h.engine(), ta).unwrap(), expected.as_str());
                    let t = get::<Textarea>(h, ta);
                    prop_assert_eq!(t.cursor_pos(), cur);
                    prop_assert_eq!(t.cursor().byte, model[..cur].iter().map(|c| c.len_utf8()).sum::<usize>());
                }
                Ok(())
            })?;
        }
    }
}

#[test]
fn textarea_defaults() {
    let mut h = harness(300, 200, Mode::Light);
    let screen = h.screen();
    let ta = textarea::create(h.engine_mut(), screen).unwrap();
    h.run_until_idle();
    let n = h.engine().tree().node(ta).unwrap();
    assert_eq!(n.class().name, "textarea");
    assert_eq!(
        TEXTAREA_CLASS.parts,
        &[
            Part::Main,
            Part::Scrollbar,
            Part::Selected,
            Part::Cursor,
            Part::CustomFirst
        ]
    );
    let c = h.engine().coords(ta);
    assert_eq!((c.width(), c.height()), (260, 130));
    let t = get::<Textarea>(&h, ta);
    assert_eq!(text(&h, ta), "");
    assert_eq!(t.cursor_pos(), 0);
    assert!(!t.password_mode() && !t.one_line() && t.cursor_click_pos() && !t.text_selection());
    assert_eq!(t.password_show_time(), textarea::DEFAULT_PWD_SHOW_TIME);
    assert_eq!(t.max_length(), 0);
    // The label child holds the text.
    let label = t.label();
    assert_eq!(h.engine().tree().parent(label), Some(ta));
    assert!(h.engine().widget::<Label>(label).is_some());
    assert!(!h.engine().has_flag(ta, twine_engine::ObjFlags::SCROLL_WITH_ARROW));
    h.assert_idle();
}

#[test]
fn cursor_blinks_only_when_focused() {
    let (mut h, ta) = scene(Mode::Light);
    with(&mut h, ta, |t: &mut Textarea, cx| t.add_text(cx, "abc"));
    h.run_until_idle();
    h.assert_idle();
    assert!(!get::<Textarea>(&h, ta).is_blinking(&MeasureCx::new(h.engine(), ta)));
    // Focused: the cursor blinks every 400 ms (the default theme's `anim_duration`).
    h.engine_mut().focus(ta);
    h.update();
    assert!(get::<Textarea>(&h, ta).is_blinking(&MeasureCx::new(h.engine(), ta)));
    assert!(get::<Textarea>(&h, ta).cursor().show);
    h.advance(Duration::ms(410));
    assert!(!get::<Textarea>(&h, ta).cursor().show);
    h.advance(Duration::ms(400));
    assert!(get::<Textarea>(&h, ta).cursor().show);
    // Unfocused again: no timer, idle.
    let g = h.engine().group_of(ta).unwrap();
    h.engine_mut().focus_next(g);
    assert!(!get::<Textarea>(&h, ta).is_blinking(&MeasureCx::new(h.engine(), ta)));
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn raw_focused_state_starts_and_stops_blink() {
    let (mut h, ta) = scene(Mode::Light);
    h.run_until_idle();
    let blinking = |h: &EngineHarness| get::<Textarea>(h, ta).is_blinking(&MeasureCx::new(h.engine(), ta));
    assert!(!blinking(&h));
    // No focus group involved: the application sets the state directly.
    h.engine_mut().add_state(ta, State::FOCUSED);
    assert!(blinking(&h), "StateChanged starts the blink");
    h.update(); // the blink period starts at this update
    h.advance(Duration::ms(410));
    assert!(!get::<Textarea>(&h, ta).cursor().show);
    h.engine_mut().clear_state(ta, State::FOCUSED);
    assert!(!blinking(&h), "StateChanged stops the blink at once");
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn blink_invalidates_cursor_area_only() {
    let (mut h, ta) = scene(Mode::Light);
    with(&mut h, ta, |t: &mut Textarea, cx| t.add_text(cx, "Hello world"));
    h.engine_mut().focus(ta);
    h.advance(Duration::ms(100));
    let area = get::<Textarea>(&h, ta).cursor_area(&MeasureCx::new(h.engine(), ta));
    assert!(!area.is_empty());
    // One blink period later exactly the cursor area is redrawn.
    let mut frames = 0;
    for _ in 0..100 {
        // One update per step (`invalidations` describe the last update).
        h.clock().advance(Duration::ms(10));
        h.update();
        let inv: Vec<Rect> = h.invalidations().iter().map(|(r, _)| *r).collect();
        if !inv.is_empty() {
            frames += 1;
            for r in &inv {
                assert!(area.contains_rect(r), "{r:?} outside the cursor {area:?}");
            }
            let dirty = h.last_frame().dirty_px;
            assert!(dirty <= u32::try_from(area.area()).unwrap(), "{dirty} px");
        }
    }
    assert!(
        (2..=3).contains(&frames),
        "{frames} redraws in ~1 s (400 ms blink)"
    );
}

#[test]
fn click_sets_cursor_pos() {
    let (mut h, ta) = scene(Mode::Light);
    with(&mut h, ta, |t: &mut Textarea, cx| t.add_text(cx, "abcdef"));
    h.run_until_idle();
    let label = get::<Textarea>(&h, ta).label();
    let l = get::<Label>(&h, label);
    let lm = MeasureCx::new(h.engine(), label);
    let lc = h.engine().content_area(label);
    let p3 = l.letter_pos(&lm, 3);
    h.tap(Point::new(lc.x0 + p3.x + 1, lc.y0 + 4));
    assert_eq!(cursor(&h, ta), 3);
    // Right of the text on its line: the nearest boundary (the end of the line).
    h.tap(Point::new(lc.x1 - 2, lc.y0 + 4));
    assert_eq!(cursor(&h, ta), 6);
    // Disabled: the cursor stays.
    with(&mut h, ta, |t: &mut Textarea, cx| {
        t.set_cursor_click_pos(cx, false);
    });
    h.tap(Point::new(lc.x0 + 1, lc.y0 + 4));
    assert_eq!(cursor(&h, ta), 6);
    h.advance(Duration::ms(50));
}

#[test]
fn drag_selects_range() {
    let (mut h, ta) = scene(Mode::Light);
    with(&mut h, ta, |t: &mut Textarea, cx| {
        t.add_text(cx, "abcdefgh");
        t.set_text_selection(cx, true);
    });
    h.run_until_idle();
    let label = get::<Textarea>(&h, ta).label();
    let lc = h.engine().content_area(label);
    let x = |i: usize| {
        let l = get::<Label>(&h, label);
        l.letter_pos(&MeasureCx::new(h.engine(), label), i).x + lc.x0 + 1
    };
    let (from, to) = (Point::new(x(2), lc.y0 + 5), Point::new(x(6), lc.y0 + 5));
    h.drag(from, to, Duration::ms(100));
    let sel = get::<Textarea>(&h, ta).selection(&MeasureCx::new(h.engine(), ta));
    assert_eq!(sel, Some((2, 6)));
    assert_eq!(cursor(&h, ta), 6);
    // A tap clears it.
    h.tap(from);
    assert!(!get::<Textarea>(&h, ta).text_is_selected(&MeasureCx::new(h.engine(), ta)));
    h.advance(Duration::ms(50));
}

#[test]
fn typing_replaces_selection() {
    let (mut h, ta) = scene(Mode::Light);
    with(&mut h, ta, |t: &mut Textarea, cx| {
        t.add_text(cx, "hello world");
        t.set_selection(cx, 0, 5);
        t.add_char(cx, 'J');
    });
    assert_eq!(text(&h, ta), "J world");
    assert_eq!(cursor(&h, ta), 1);
    with(&mut h, ta, |t: &mut Textarea, cx| {
        t.set_selection(cx, 1, 7);
        t.delete_char(cx);
    });
    assert_eq!(text(&h, ta), "J", "backspace deletes the selection");
}

#[test]
fn password_hides_after_show_time() {
    let (mut h, ta) = scene(Mode::Light);
    with(&mut h, ta, |t: &mut Textarea, cx| {
        t.set_password_mode(cx, true);
        t.add_text(cx, "se");
        t.add_char(cx, 'c');
    });
    let label = get::<Textarea>(&h, ta).label();
    let shown = |h: &EngineHarness| get::<Label>(h, label).shown_text().to_owned();
    assert_eq!(shown(&h), "••c", "the last typed character is visible");
    h.advance(Duration::ms(1400));
    assert_eq!(shown(&h), "••c");
    h.advance(Duration::ms(200));
    assert_eq!(shown(&h), "•••");
    h.run_until_idle();
    h.assert_idle();
    // Show time 0: hidden at once; a custom bullet.
    with(&mut h, ta, |t: &mut Textarea, cx| {
        t.set_password_show_time(cx, Duration::ZERO);
        t.set_password_bullet(cx, Some("*"));
        t.add_char(cx, 'r');
    });
    assert_eq!(shown(&h), "****");
    with(&mut h, ta, |t: &mut Textarea, cx| t.set_password_mode(cx, false));
    assert_eq!(shown(&h), "secr");
}

#[test]
fn password_text_returns_real() {
    let (mut h, ta) = scene(Mode::Light);
    let log = value_changes(&mut h, ta);
    with(&mut h, ta, |t: &mut Textarea, cx| {
        t.set_password_mode(cx, true);
        t.add_text(cx, "pa55");
    });
    assert_eq!(text(&h, ta), "pa55");
    let t = get::<Textarea>(&h, ta);
    assert_eq!(t.text(&MeasureCx::new(h.engine(), ta)), "pa55");
    assert_eq!(*log.borrow(), vec!["pa55"], "handlers see the real text");
    // Tree dumps and queries see only the bullets.
    assert!(!h.tree_dump().contains("pa55"));
}

#[test]
fn scroll_follows_cursor_multiline() {
    let (mut h, ta) = scene(Mode::Light);
    let _ = h.keypad_input();
    h.engine_mut().focus(ta);
    for i in 0..10 {
        h.type_text(&format!("line {i}"));
        h.key(Key::Enter);
    }
    h.advance(Duration::ms(100));
    let top = h.engine().scroll_offset(ta).y;
    assert!(top > 0, "scrolled down to the cursor");
    let cur = get::<Textarea>(&h, ta).cursor_area(&MeasureCx::new(h.engine(), ta));
    let c = h.engine().coords(ta);
    assert!(c.contains_rect(&cur), "cursor {cur:?} visible in {c:?}");
    // Back to the start: scrolled up again.
    h.key(Key::Home);
    for _ in 0..10 {
        h.key(Key::Up);
    }
    h.advance(Duration::ms(100));
    assert_eq!(h.engine().scroll_offset(ta).y, 0);
}

#[test]
fn one_line_scrolls_horizontally() {
    let (mut h, ta) = scene(Mode::Light);
    with(&mut h, ta, |t: &mut Textarea, cx| {
        t.set_one_line(cx, true);
        t.add_text(cx, "a rather long line that does not fit into the field");
    });
    h.advance(Duration::ms(100));
    let x = h.engine().scroll_offset(ta).x;
    assert!(x > 0, "scrolled right to the cursor");
    let cur = get::<Textarea>(&h, ta).cursor_area(&MeasureCx::new(h.engine(), ta));
    assert!(h.engine().coords(ta).contains_rect(&cur));
    with(&mut h, ta, |t: &mut Textarea, cx| t.set_cursor_pos(cx, 0));
    h.advance(Duration::ms(100));
    assert_eq!(h.engine().scroll_offset(ta).x, 0);
}

#[test]
fn placeholder_shown_when_empty() {
    let (mut h, ta) = scene(Mode::Light);
    with(&mut h, ta, |t: &mut Textarea, cx| {
        t.set_placeholder_text(cx, "Your name");
    });
    h.run_until_idle();
    let lc = h.engine().content_area(get::<Textarea>(&h, ta).label());
    // Some placeholder pixel differs from the background.
    let bg = h.pixel(
        u32::try_from(lc.x0 + 60).unwrap(),
        u32::try_from(lc.y0 - 4).unwrap(),
    );
    let has_text = |h: &EngineHarness| {
        (lc.x0..lc.x0 + 60).any(|x| (lc.y0..lc.y0 + 16).any(|y| h.pixel(x as u32, y as u32) != bg))
    };
    assert!(has_text(&h));
    assert_eq!(get::<Textarea>(&h, ta).placeholder_text(), "Your name");
    assert_eq!(text(&h, ta), "", "the placeholder is not the text");
    // The placeholder's color comes from the placeholder part (grey).
    let c = h
        .engine()
        .style_color(ta, textarea::PLACEHOLDER, PropId::TextColor);
    assert_eq!(c, twine_theme::Palette::Grey.lighten(1));
    with(&mut h, ta, |t: &mut Textarea, cx| t.add_char(cx, ' '));
    h.run_until_idle();
    assert!(!has_text(&h), "a space hides the placeholder");
}

#[test]
fn textarea_keypad_and_encoder() {
    let (mut h, ta) = scene(Mode::Light);
    let screen = h.screen();
    let _other = textarea::create(h.engine_mut(), screen).unwrap();
    let g = h.engine().group_of(ta).unwrap();
    let _ = h.keypad_input();
    h.engine_mut().focus(ta);
    h.type_text("ab");
    h.key(Key::Left);
    h.type_text("X");
    assert_eq!(text(&h, ta), "aXb");
    // Encoder: a click enters edit mode, turning moves the cursor.
    let _ = h.encoder_input();
    h.encoder_click();
    assert!(h.engine().group_editing(g));
    h.encoder(-2);
    assert_eq!(cursor(&h, ta), 0);
    h.encoder(3);
    assert_eq!(cursor(&h, ta), 3);
    h.advance(Duration::ms(50));
}

#[test]
fn textarea_idle_after_interaction() {
    let (mut h, ta) = scene(Mode::Dark);
    let c = h.engine().coords(ta);
    h.tap(Point::new(c.x0 + 20, c.y0 + 20));
    let _ = h.keypad_input();
    h.type_text("hi");
    // Focus another widget: the textarea stops blinking and everything settles.
    let screen = h.screen();
    let other = twine_widgets::button::create(h.engine_mut(), screen).unwrap();
    h.engine_mut().focus(other);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn textarea_theme_styles() {
    let (mut h, ta) = scene(Mode::Light);
    let e = h.engine();
    let cursor_sel = |h: &EngineHarness| h.engine().style_i32(ta, Part::Cursor, PropId::BorderWidth);
    // The cursor style applies only while focused.
    assert_eq!(cursor_sel(&h), 0);
    assert_eq!(e.style_i32(ta, Part::Cursor, PropId::AnimDuration), 0);
    h.engine_mut().add_state(ta, State::FOCUSED);
    assert_eq!(cursor_sel(&h), 2);
    assert_eq!(h.engine().style_i32(ta, Part::Cursor, PropId::AnimDuration), 400);
    // The label's selection is primary.
    let label = get::<Textarea>(&h, ta).label();
    assert_eq!(
        h.engine().style_color(label, Part::Selected, PropId::BgColor),
        twine_theme::Palette::Blue.main()
    );
    h.engine_mut().clear_state(ta, State::FOCUSED);
}

#[test]
fn snapshot_textarea_states() {
    for mode in Mode::ALL {
        let sfx = mode.suffix();
        let (mut h, ta) = scene(mode);
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.set_placeholder_text(cx, "Your name");
        });
        h.run_until_idle();
        h.assert_snapshot(&format!("textarea_empty_placeholder_{sfx}"));

        let (mut h, ta) = scene(mode);
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.add_text(cx, "The quick brown fox jumps over the lazy dog.");
        });
        h.run_until_idle();
        h.assert_snapshot(&format!("textarea_text_{sfx}"));

        // Focused (keypad): the outline and the cursor.
        let _ = h.keypad_input();
        h.key(Key::Next);
        assert!(
            h.engine()
                .tree()
                .node(ta)
                .unwrap()
                .state()
                .contains(State::FOCUS_KEY)
        );
        h.key(Key::Left);
        // The cursor blinks (never idle): redraw everything and capture the panel at once.
        h.engine_mut().invalidate_all();
        h.advance(Duration::ms(40));
        assert!(get::<Textarea>(&h, ta).cursor().show);
        h.assert_panel_snapshot(&format!("textarea_focused_cursor_{sfx}"));

        let (mut h, ta) = scene(mode);
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.add_text(cx, "Select some text");
            t.set_selection(cx, 7, 11);
        });
        h.run_until_idle();
        h.assert_snapshot(&format!("textarea_selection_{sfx}"));

        let (mut h, ta) = scene(mode);
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.set_one_line(cx, true);
            t.set_password_mode(cx, true);
            t.add_text(cx, "secret");
        });
        h.run_until_idle();
        h.assert_snapshot(&format!("textarea_password_{sfx}"));
    }
}
