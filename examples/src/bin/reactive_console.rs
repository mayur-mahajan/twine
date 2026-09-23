//! `reactive_console`: a console walk-through of the reactive runtime (no window).
//!
//! Builds signals `a`, `b`, memos `sum = a + b` and `parity = sum % 2`, and two effects that print
//! them, then mutates the graph step by step and prints exactly which memos and effects ran:
//! glitch-freedom, memo short-circuit, batching, disposal and channel delivery from a thread.
//!
//! ```text
//! cargo xtask sim reactive_console
//! RUST_LOG=twine::reactive=trace cargo xtask sim reactive_console   # also log every effect run
//! ```

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use twine_reactive::{Channel, batch, create_root, drain_channels};

/// Values sent to the UI thread from a worker thread.
static INPUT: Channel<i32, 8> = Channel::new();

/// What ran during one step, in order.
type Trace = Rc<RefCell<Vec<String>>>;

fn record(trace: &Trace, line: String) {
    println!("      {line}");
    trace.borrow_mut().push(line);
}

/// Prints the step header, runs `f`, then prints and returns what ran.
fn step(n: u32, title: &str, trace: &Trace, f: impl FnOnce()) -> Vec<String> {
    println!("[{n}] {title}");
    trace.borrow_mut().clear();
    f();
    let ran = trace.borrow().clone();
    let effects: Vec<_> = ran
        .iter()
        .filter(|l| l.starts_with('E'))
        .map(|l| &l[..2])
        .collect();
    println!(
        "    -> effects run: {}",
        if effects.is_empty() {
            "none".to_string()
        } else {
            effects.join(", ")
        }
    );
    ran
}

fn count(ran: &[String], prefix: &str) -> usize {
    ran.iter().filter(|l| l.starts_with(prefix)).count()
}

fn main() {
    env_logger::init();
    let trace: Trace = Rc::default();
    let root = create_root();
    let effects_scope = root.child();

    let (a, b) = (root.signal(1), root.signal(2));
    let t = trace.clone();
    let sum = root.memo(move || {
        let v = a.get() + b.get();
        record(&t, format!("memo sum    = {v}"));
        v
    });
    let t = trace.clone();
    let parity = root.memo(move || {
        let v = sum.get() % 2;
        record(&t, format!("memo parity = {v}"));
        v
    });

    let ran = step(
        1,
        "create a=1, b=2, sum=a+b, parity=sum%2, effects E1(sum) and E2(parity)",
        &trace,
        || {
            let t = trace.clone();
            root.effect(move || record(&t, format!("E1 sum    = {}", sum.get())));
            let t = trace.clone();
            effects_scope.effect(move || record(&t, format!("E2 parity = {}", parity.get())));
        },
    );
    assert_eq!((count(&ran, "E1"), count(&ran, "E2")), (1, 1));

    let ran = step(2, "a.set(2): sum 3 -> 4, parity 1 -> 0", &trace, || a.set(2));
    assert_eq!((count(&ran, "E1"), count(&ran, "E2")), (1, 1));
    assert_eq!(count(&ran, "memo sum"), 1, "glitch-free: sum computed once");

    let ran = step(
        3,
        "a.set(4): sum 4 -> 6, parity stays 0 (memo short-circuit)",
        &trace,
        || a.set(4),
    );
    assert_eq!((count(&ran, "E1"), count(&ran, "E2")), (1, 0));
    assert_eq!(count(&ran, "memo parity"), 1);

    let ran = step(
        4,
        "batch(|| { a.set(1); b.set(1); }): one flush for two writes",
        &trace,
        || {
            batch(|| {
                a.set(1);
                b.set(1);
            });
        },
    );
    assert_eq!((count(&ran, "E1"), count(&ran, "E2")), (1, 0));

    let ran = step(
        5,
        "dispose the child scope holding E2, then a.set(2)",
        &trace,
        || {
            effects_scope.dispose();
            a.set(2);
        },
    );
    assert_eq!((count(&ran, "E1"), count(&ran, "E2")), (1, 0));
    assert!(!effects_scope.is_alive());

    let ran = step(
        6,
        "a worker thread sends 10 on a Channel; the handler sets a",
        &trace,
        || {
            root.on_message(&INPUT, move |v| a.set(v));
            let worker = std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(20));
                INPUT.try_send(10).expect("channel has room");
            });
            // A UI loop would sleep until woken; poll the channel's wake flag here.
            while !INPUT.waker().take() {
                std::thread::sleep(Duration::from_millis(1));
            }
            let n = drain_channels(16);
            println!("      drained {n} message(s)");
            worker.join().expect("worker thread");
        },
    );
    assert_eq!(count(&ran, "E1"), 1);
    assert_eq!(a.get(), 10);

    root.dispose();
    let stats = twine_reactive::debug_stats();
    println!(
        "done: {} nodes and {} scopes left after disposing the root",
        stats.nodes, stats.scopes
    );
}
