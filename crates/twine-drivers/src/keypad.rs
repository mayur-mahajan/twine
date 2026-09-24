//! Key matrix scanned over GPIOs: [`GpioMatrix`] (`InputKind::Keypad`).
//!
//! # Scanning
//!
//! Rows are outputs, idle **high**; columns are inputs with pull-ups. A scan drives one row low
//! at a time and reads the columns: a column reading low means the key at (row, column) is
//! pressed. Every key is debounced separately (default 20 ms, using the clock given at
//! construction) and each debounced change is queued as a key event (queue of 8; the engine
//! reads them one by one, [`KeypadData::more`] tells it to read again).
//!
//! # Ghosting
//!
//! Without a diode per key, pressing three keys at three corners of a rectangle makes the
//! fourth corner read as pressed (current flows back through the pressed keys). Two keys are
//! always reported correctly. Use diodes (cathode towards the row) or only rely on single keys
//! and pairs. Configure rows as open-drain outputs if several keys in one column may be pressed
//! at once, so that two driven rows never short.
//!
//! # Wiring (example: 4 × 4 membrane keypad on an RP2040)
//!
//! | Keypad | GPIO |
//! |--------|------|
//! | R1…R4 | GP2…GP5 (outputs, open drain or push-pull) → `rows` |
//! | C1…C4 | GP6…GP9 (inputs, pull-up) → `cols` |
//!
//! ```
//! # use core::cell::Cell;
//! # use twine_core::Instant;
//! # use twine_hal::Clock;
//! # struct Now(Cell<Instant>);
//! # impl Clock for Now { fn now(&self) -> Instant { self.0.get() } }
//! use twine_drivers::keypad::GpioMatrix;
//! use twine_drivers::testkit::Recorder;
//! use twine_hal::{InputData, InputDevice, Key};
//!
//! let rec = Recorder::new();
//! let clock = Now(Cell::new(Instant::from_millis(0)));
//! let rows = [rec.quiet_pin("r0"), rec.quiet_pin("r1")];
//! let cols = [rec.quiet_pin("c0"), rec.quiet_pin("c1")];
//! rec.set_level("c0", true);
//! rec.set_level("c1", true); // no key pressed: columns pulled up
//! let keymap = [[Key::Up, Key::Enter], [Key::Down, Key::Esc]];
//! let mut pad = GpioMatrix::new(rows, cols, keymap, &clock);
//! assert!(matches!(pad.read(), InputData::Keypad(k) if !k.pressed));
//! ```

use embedded_hal::digital::{InputPin, OutputPin};
use heapless::Deque;
use twine_core::Duration;
use twine_core::log::{debug, warn};
use twine_hal::{Clock, InputData, InputDevice, InputKind, Key, KeypadData};

use crate::debounce::Debouncer;

/// Capacity of the event queue.
pub const QUEUE_LEN: usize = 8;

/// A `R × C` key matrix implementing [`InputDevice`] (`Keypad`).
#[derive(Debug)]
pub struct GpioMatrix<ROW, COL, C, const R: usize, const N: usize> {
    rows: [ROW; R],
    cols: [COL; N],
    keymap: [[Key; N]; R],
    keys: [[Debouncer; N]; R],
    queue: Deque<(Key, bool), QUEUE_LEN>,
    last: KeypadData,
    clock: C,
}

impl<ROW: OutputPin, COL: InputPin, C: Clock, const R: usize, const N: usize> GpioMatrix<ROW, COL, C, R, N> {
    /// A matrix with `keymap[row][column]`; drives every row high (idle).
    #[must_use]
    pub fn new(mut rows: [ROW; R], cols: [COL; N], keymap: [[Key; N]; R], clock: C) -> Self {
        for r in &mut rows {
            if r.set_high().is_err() {
                warn!(target: "twine::driver", "keypad: row pin error");
            }
        }
        Self {
            rows,
            cols,
            keymap,
            keys: [[Debouncer::default(); N]; R],
            queue: Deque::new(),
            last: KeypadData {
                key: Key::Enter,
                pressed: false,
                more: false,
            },
            clock,
        }
    }

    /// Sets the per-key debounce time.
    #[must_use]
    pub fn with_debounce(mut self, debounce: Duration) -> Self {
        self.keys = [[Debouncer::new(debounce); N]; R];
        self
    }

    /// Scans the matrix once and queues debounced changes. `read` scans too; call this from a
    /// periodic task if the engine reads less often than the debounce time.
    pub fn scan(&mut self) {
        let now = self.clock.now();
        for r in 0..R {
            if self.rows[r].set_low().is_err() {
                warn!(target: "twine::driver", "keypad: row pin error");
                continue;
            }
            for c in 0..N {
                let raw = self.cols[c].is_low().unwrap_or(false);
                let before = self.keys[r][c].level();
                let after = self.keys[r][c].update(raw, now);
                if before != after {
                    let key = self.keymap[r][c];
                    debug!(target: "twine::driver", "keypad {} {}", key, if after { "down" } else { "up" });
                    if self.queue.push_back((key, after)).is_err() {
                        warn!(target: "twine::driver", "keypad: event queue full, dropped {}", key);
                    }
                }
            }
            if self.rows[r].set_high().is_err() {
                warn!(target: "twine::driver", "keypad: row pin error");
            }
        }
    }

    /// Returns the pins and the clock.
    #[must_use]
    pub fn release(self) -> ([ROW; R], [COL; N], C) {
        (self.rows, self.cols, self.clock)
    }
}

impl<ROW: OutputPin, COL: InputPin, C: Clock, const R: usize, const N: usize> InputDevice
    for GpioMatrix<ROW, COL, C, R, N>
{
    fn kind(&self) -> InputKind {
        InputKind::Keypad
    }

    /// Returns the oldest queued event (`more` = further events are queued), or the last key's
    /// current state when nothing happened.
    fn read(&mut self) -> InputData {
        if self.queue.is_empty() {
            self.scan();
        }
        if let Some((key, pressed)) = self.queue.pop_front() {
            self.last = KeypadData {
                key,
                pressed,
                more: !self.queue.is_empty(),
            };
        } else {
            self.last.more = false;
        }
        InputData::Keypad(self.last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use core::cell::{Cell, RefCell};
    use core::convert::Infallible;
    use embedded_hal::digital::ErrorType;
    use twine_core::Instant;

    /// A simulated 3 × 3 matrix: pressed keys connect their row to their column.
    #[derive(Default)]
    struct Matrix {
        rows_low: [bool; 3],
        pressed: Vec<(usize, usize)>,
    }

    struct Row(Rc<RefCell<Matrix>>, usize);
    struct Col(Rc<RefCell<Matrix>>, usize);

    impl ErrorType for Row {
        type Error = Infallible;
    }
    impl OutputPin for Row {
        fn set_low(&mut self) -> Result<(), Infallible> {
            self.0.borrow_mut().rows_low[self.1] = true;
            Ok(())
        }
        fn set_high(&mut self) -> Result<(), Infallible> {
            self.0.borrow_mut().rows_low[self.1] = false;
            Ok(())
        }
    }
    impl ErrorType for Col {
        type Error = Infallible;
    }
    impl InputPin for Col {
        fn is_high(&mut self) -> Result<bool, Infallible> {
            self.is_low().map(|l| !l)
        }
        fn is_low(&mut self) -> Result<bool, Infallible> {
            let m = self.0.borrow();
            Ok(m.pressed.iter().any(|&(r, c)| c == self.1 && m.rows_low[r]))
        }
    }

    struct TestClock(Cell<Instant>);
    impl Clock for TestClock {
        fn now(&self) -> Instant {
            self.0.get()
        }
    }

    const KEYMAP: [[Key; 3]; 3] = [
        [Key::Char('1'), Key::Char('2'), Key::Char('3')],
        [Key::Char('4'), Key::Char('5'), Key::Char('6')],
        [Key::Left, Key::Enter, Key::Right],
    ];

    fn setup(clock: &TestClock) -> (Rc<RefCell<Matrix>>, GpioMatrix<Row, Col, &TestClock, 3, 3>) {
        let m = Rc::new(RefCell::new(Matrix::default()));
        let rows = core::array::from_fn(|i| Row(m.clone(), i));
        let cols = core::array::from_fn(|i| Col(m.clone(), i));
        (m, GpioMatrix::new(rows, cols, KEYMAP, clock))
    }

    fn keypad(d: InputData) -> KeypadData {
        match d {
            InputData::Keypad(k) => k,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn matrix_detects_single_key() {
        let clock = TestClock(Cell::new(Instant::from_millis(0)));
        let (m, mut pad) = setup(&clock);
        assert!(!keypad(pad.read()).pressed);
        m.borrow_mut().pressed.push((1, 2));
        assert!(!keypad(pad.read()).pressed); // not yet debounced
        clock.0.set(Instant::from_millis(20));
        assert_eq!(
            keypad(pad.read()),
            KeypadData {
                key: Key::Char('6'),
                pressed: true,
                more: false
            }
        );
        // Held: no new event, state stays pressed.
        clock.0.set(Instant::from_millis(30));
        assert_eq!(
            keypad(pad.read()),
            KeypadData {
                key: Key::Char('6'),
                pressed: true,
                more: false
            }
        );
        m.borrow_mut().pressed.clear();
        let _ = pad.read();
        clock.0.set(Instant::from_millis(50));
        assert_eq!(
            keypad(pad.read()),
            KeypadData {
                key: Key::Char('6'),
                pressed: false,
                more: false
            }
        );
        assert_eq!(pad.kind(), InputKind::Keypad);
        // All rows idle high after scanning.
        assert_eq!(m.borrow().rows_low, [false; 3]);
    }

    #[test]
    fn matrix_ghosting_documented_two_keys() {
        let clock = TestClock(Cell::new(Instant::from_millis(0)));
        let (m, mut pad) = setup(&clock).pipe_debounce();
        m.borrow_mut().pressed.extend([(0, 0), (2, 1)]);
        let first = keypad(pad.read());
        let second = keypad(pad.read());
        assert_eq!(
            first,
            KeypadData {
                key: Key::Char('1'),
                pressed: true,
                more: true
            }
        );
        assert_eq!(
            second,
            KeypadData {
                key: Key::Enter,
                pressed: true,
                more: false
            }
        );
    }

    #[test]
    fn keypad_queue_reports_more_flag() {
        let clock = TestClock(Cell::new(Instant::from_millis(0)));
        let (m, mut pad) = setup(&clock).pipe_debounce();
        m.borrow_mut().pressed.extend([(0, 1), (1, 1), (2, 2)]);
        let evs: Vec<_> = (0..3).map(|_| keypad(pad.read())).collect();
        assert_eq!(
            evs.iter().map(|e| e.more).collect::<Vec<_>>(),
            [true, true, false]
        );
        assert_eq!(
            evs.iter().map(|e| e.key).collect::<Vec<_>>(),
            [Key::Char('2'), Key::Char('5'), Key::Right]
        );
        let _ = pad.release();
    }

    trait PipeDebounce {
        fn pipe_debounce(self) -> Self;
    }
    impl PipeDebounce for (Rc<RefCell<Matrix>>, GpioMatrix<Row, Col, &TestClock, 3, 3>) {
        /// No debounce: changes are reported on the first scan.
        fn pipe_debounce(self) -> Self {
            (self.0, self.1.with_debounce(Duration::ms(0)))
        }
    }
}
