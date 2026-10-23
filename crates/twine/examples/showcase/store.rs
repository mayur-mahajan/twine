//! The monitor's application state (the store pattern of `twine::guide::app_state`): the
//! signals of [`MonitorState`] behind one `Copy` handle ([`Monitor`]), provided as context, read
//! by every screen.
//!
//! **Two rates.** A real monitor separates the *waveform* path (hundreds of samples per
//! second, drawn as they arrive) from the *numeric* path (values averaged and published about
//! once a second, alarms evaluated on them). The store does the same with one engine timer
//! running every 16 ms (60 Hz):
//!
//! - every tick: 4 simulator steps → one min/max column per waveform pushed into its
//!   [`Trace`] ring, the trace heads published (three `Signal<u32>` writes), the heart-beat
//!   flag and the alarm flash phases updated. These are plain integer signals written with
//!   `set`/`set_if_changed`: no allocation, and only the bindings that read them re-run
//!   (the three waveform setters, the heart icon, the alarm visuals);
//! - every simulated second: the numerics, the clock, the alarm evaluation against the
//!   limits, the audio-pause countdown and the NIBP cuff;
//! - every [`TREND_STEP_S`] seconds: one trend point.
//!
//! The UI writes back through action methods (`silence`, `start_nibp`, `set_limits`, …),
//! which the tick picks up — the same one-way flow a sensor task and a UI task would have.
//!
//! **Alarms** follow the shape of IEC 60601-1-8: each limit crossing is an alarm *condition*
//! with a priority (high: SpO2 low, HR high/low; medium: RR, NIBP), raised when the numeric
//! crosses its limit and cleared when it returns (non-latching), logged with its start, end and
//! worst value. *Audio paused* ("silence 2 min") lasts [`PAUSE_S`] seconds and ends early when
//! a new condition arises, so a new alarm is never silent. Visual signals keep flashing while
//! audio is paused: red at 2 Hz for high priority, yellow at 0.5 Hz for medium.

use std::collections::VecDeque;

use twine::prelude::*;
use twine::reactive::StoredValue;

use crate::patient::{Nibp, Patient, TREND_CAPACITY, TREND_STEP_S, TrendPoint, trend_history};
use crate::waveform::{Envelope, Trace};

/// The simulator's seed: the same seed gives the same patient, frame for frame.
pub const SEED: u32 = 0x00C0_FFEE;
/// The engine timer period (one waveform column per tick: 62.5 columns per second).
pub const TICK_MS: u32 = 16;
/// Length of an audio pause, seconds.
pub const PAUSE_S: u32 = 120;
/// The wall-clock time the demo starts at (14:32:00), seconds since midnight.
const START_CLOCK: u32 = 14 * 3600 + 32 * 60;
/// Alarm history entries kept (the oldest are dropped: bounded memory).
const LOG_CAPACITY: usize = 32;

/// Alarm priority (IEC 60601-1-8 distinguishes high, medium and low).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// Prompt response required: yellow, flashing slowly.
    Medium,
    /// Immediate response required: red, flashing fast.
    High,
}

/// A physiological alarm condition: a numeric beyond one of its limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Condition {
    HrHigh,
    HrLow,
    Spo2Low,
    RrHigh,
    RrLow,
    NibpHigh,
    NibpLow,
}

impl Condition {
    /// Every condition the monitor checks.
    pub const ALL: [Condition; 7] = [
        Condition::Spo2Low,
        Condition::HrHigh,
        Condition::HrLow,
        Condition::RrHigh,
        Condition::RrLow,
        Condition::NibpHigh,
        Condition::NibpLow,
    ];

    /// The priority of the condition.
    pub const fn priority(self) -> Priority {
        match self {
            Condition::Spo2Low | Condition::HrHigh | Condition::HrLow => Priority::High,
            _ => Priority::Medium,
        }
    }

    /// The alarm message, as on the banner.
    pub const fn message(self) -> &'static str {
        match self {
            Condition::HrHigh => "HR HIGH",
            Condition::HrLow => "HR LOW",
            Condition::Spo2Low => "SpO2 LOW",
            Condition::RrHigh => "RR HIGH",
            Condition::RrLow => "RR LOW",
            Condition::NibpHigh => "NIBP SYS HIGH",
            Condition::NibpLow => "NIBP SYS LOW",
        }
    }

    /// The parameter the condition is about (to highlight its tile).
    pub const fn param(self) -> Param {
        match self {
            Condition::HrHigh | Condition::HrLow => Param::Hr,
            Condition::Spo2Low => Param::Spo2,
            Condition::RrHigh | Condition::RrLow => Param::Rr,
            Condition::NibpHigh | Condition::NibpLow => Param::Nibp,
        }
    }

    /// The current value and the limit it is compared with; `Some` when the limit is crossed.
    fn check(self, v: &Vitals, l: &AlarmLimits) -> Option<(i32, i32)> {
        let (value, limit, crossed) = match self {
            Condition::HrHigh => (v.hr, l.hr_high, v.hr > l.hr_high),
            Condition::HrLow => (v.hr, l.hr_low, v.hr < l.hr_low),
            Condition::Spo2Low => (v.spo2, l.spo2_low, v.spo2 < l.spo2_low),
            Condition::RrHigh => (v.rr, l.rr_high, v.rr > l.rr_high),
            Condition::RrLow => (v.rr, l.rr_low, v.rr < l.rr_low),
            Condition::NibpHigh => (v.sys, l.sys_high, v.sys > l.sys_high),
            Condition::NibpLow => (v.sys, l.sys_low, v.sys < l.sys_low),
        };
        crossed.then_some((value, limit))
    }
}

/// A monitored parameter (a tile on the monitor screen).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Param {
    Hr,
    Spo2,
    Rr,
    Nibp,
}

/// An active alarm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alarm {
    pub cond: Condition,
    /// The current value of the parameter.
    pub value: i32,
    /// The limit crossed.
    pub limit: i32,
    /// When it started (clock seconds).
    pub since: u32,
}

/// An entry of the alarm history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    /// Unique, for the keyed list.
    pub id: u32,
    pub cond: Condition,
    /// Start and end (clock seconds; `None` while active).
    pub start: u32,
    pub end: Option<u32>,
    /// The value furthest beyond the limit.
    pub worst: i32,
}

/// The alarm limits (Setup screen).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlarmLimits {
    pub hr_low: i32,
    pub hr_high: i32,
    pub spo2_low: i32,
    pub rr_low: i32,
    pub rr_high: i32,
    pub sys_low: i32,
    pub sys_high: i32,
}

impl AlarmLimits {
    /// Adult defaults.
    pub const ADULT: AlarmLimits = AlarmLimits {
        hr_low: 50,
        hr_high: 120,
        spo2_low: 90,
        rr_low: 8,
        rr_high: 24,
        sys_low: 90,
        sys_high: 160,
    };
}

/// The numerics the alarms are evaluated on.
#[derive(Clone, Copy, Debug)]
struct Vitals {
    hr: i32,
    spo2: i32,
    rr: i32,
    sys: i32,
}

/// The trend history: [`TREND_CAPACITY`] points, oldest first. A scope-owned `StoredValue`
/// like the waveform rings: written by the tick, read by the trend plots while drawing.
#[derive(Debug)]
pub struct Trends {
    pub points: VecDeque<TrendPoint>,
}

/// The store: a `Copy` handle to the [`MonitorState`], provided as context and captured by
/// every binding of every screen. Read a field with its accessor: `m.hr().get()`.
///
/// **Why a handle, not the struct itself.** The state is ~25 signal handles (400 bytes). Each
/// closure that captures a `Copy` struct gets its own copy, and there are dozens of such
/// closures, so the state lives in a scope-owned [`StoredValue`] (dropped with the app's
/// scope, nothing leaked) and closures capture only its handle (the size of one signal handle). The accessors copy one
/// field (one `Copy` handle) out of it.
#[derive(Clone, Copy)]
pub struct Monitor(StoredValue<MonitorState>);

/// Generates one accessor per field of [`MonitorState`]: `m.hr()` is `m.0.with(|s| s.hr)`.
macro_rules! accessors {
    ($($field:ident: $ty:ty),* $(,)?) => {
        impl Monitor {
            $(
                #[doc = concat!("[`MonitorState::", stringify!($field), "`].")]
                pub fn $field(self) -> $ty {
                    self.0.with(|s| s.$field)
                }
            )*
        }
    };
}

accessors! {
    ecg: Trace, pleth: Trace, resp: Trace, pulse: Signal<bool>, flash_high: Signal<bool>,
    flash_med: Signal<bool>, hr: Signal<i32>, spo2: Signal<i32>, rr: Signal<i32>,
    temp: Signal<i32>, nibp: Signal<Nibp>, nibp_at: Signal<u32>, cuff: Signal<Option<i32>>,
    nibp_every: Signal<u32>, clock: Signal<u32>, limits: Signal<AlarmLimits>,
    alarms: Signal<Vec<Alarm>>, log: Signal<Vec<LogEntry>>, paused: Signal<u32>,
    trends: StoredValue<Trends>, trend_rev: Signal<u32>, patient_name: Signal<String>,
    patient_id: Signal<String>,
}

/// The signals of the store. Every field is a `Copy` handle.
#[derive(Clone, Copy)]
pub struct MonitorState {
    // The waveform path (60 Hz).
    pub ecg: Trace,
    pub pleth: Trace,
    pub resp: Trace,
    /// True for 150 ms after each R wave (the heart symbol flashes).
    pub pulse: Signal<bool>,
    /// The alarm flash phases: on/off at 2 Hz (high priority) and 0.5 Hz (medium).
    pub flash_high: Signal<bool>,
    pub flash_med: Signal<bool>,

    // The numeric path (1 Hz).
    pub hr: Signal<i32>,
    pub spo2: Signal<i32>,
    pub rr: Signal<i32>,
    /// Tenths of °C.
    pub temp: Signal<i32>,
    pub nibp: Signal<Nibp>,
    /// When the last NIBP was taken (clock seconds).
    pub nibp_at: Signal<u32>,
    /// The cuff pressure while a measurement runs.
    pub cuff: Signal<Option<i32>>,
    /// Minutes between automatic NIBP measurements (0: off).
    pub nibp_every: Signal<u32>,
    /// Seconds since midnight.
    pub clock: Signal<u32>,

    // Alarms.
    pub limits: Signal<AlarmLimits>,
    pub alarms: Signal<Vec<Alarm>>,
    pub log: Signal<Vec<LogEntry>>,
    /// Seconds of audio pause left (0: not paused).
    pub paused: Signal<u32>,

    // Trends: the history and a revision bumped on every new point.
    pub trends: StoredValue<Trends>,
    pub trend_rev: Signal<u32>,

    // The patient.
    pub patient_name: Signal<String>,
    pub patient_id: Signal<String>,

    /// Requests from the UI to the tick (private: the UI calls [`Monitor::start_nibp`]).
    nibp_request: Signal<bool>,
}

impl Monitor {
    /// Creates the store, seeds the patient and starts the 60 Hz tick in `cx`.
    pub fn new(cx: Scope) -> Self {
        let mut patient = Patient::new(SEED);
        let first = patient.numerics();
        let nibp = patient.measure_nibp();
        let mut history = VecDeque::with_capacity(TREND_CAPACITY + 1);
        history.extend(trend_history(SEED));
        let m = Monitor(cx.stored_value(MonitorState {
            ecg: Trace::new(cx),
            pleth: Trace::new(cx),
            resp: Trace::new(cx),
            pulse: cx.signal(false),
            flash_high: cx.signal(true),
            flash_med: cx.signal(true),
            hr: cx.signal(first.hr),
            spo2: cx.signal(first.spo2),
            rr: cx.signal(first.rr),
            temp: cx.signal(first.temp),
            nibp: cx.signal(nibp),
            nibp_at: cx.signal(START_CLOCK - 7 * 60),
            cuff: cx.signal(None),
            nibp_every: cx.signal(15),
            clock: cx.signal(START_CLOCK),
            limits: cx.signal(AlarmLimits::ADULT),
            alarms: cx.signal(Vec::new()),
            log: cx.signal(Vec::new()),
            paused: cx.signal(0),
            trends: cx.stored_value(Trends { points: history }),
            trend_rev: cx.signal(0),
            patient_name: cx.signal("BAKER, Ellen".into()),
            patient_id: cx.signal("00427781".into()),
            nibp_request: cx.signal(false),
        }));
        m.start(cx, patient);
        m
    }

    /// The tick: see the module docs. The closure owns the simulator and the envelopes being
    /// filled; everything the UI sees goes out through signals.
    fn start(self, cx: Scope, mut patient: Patient) {
        let mut ticks = 0u32;
        let mut beat_at = 0u32;
        let mut resp_col: Option<Envelope> = None;
        let mut nibp_started: Option<u32> = None;
        cx.interval(Duration::ms(TICK_MS.into()), move || {
            ticks += 1;
            // Four model steps (4 ms each) per column. Each column keeps the min/max of its
            // samples, so a QRS spike survives the decimation at full height.
            let (mut ecg, mut pleth) = (None, None);
            for _ in 0..4 {
                let s = patient.step();
                widen(&mut ecg, s.ecg);
                widen(&mut pleth, s.pleth);
                // Respiration sweeps at half the speed: one column per two ticks.
                widen(&mut resp_col, s.resp);
                if s.beat {
                    beat_at = patient.time_ms();
                }
            }
            let t = patient.time_ms();
            for (trace, col) in [(self.ecg(), ecg), (self.pleth(), pleth)] {
                if let Some(col) = col {
                    trace.push(col);
                }
            }
            if ticks % 2 == 0 {
                if let Some(col) = resp_col.take() {
                    self.resp().push(col);
                }
            }
            self.pulse().set_if_changed(t.wrapping_sub(beat_at) < 150);
            self.flash_high().set_if_changed(t / 250 % 2 == 0);
            self.flash_med().set_if_changed(t / 1000 % 2 == 0);

            // The numeric path, once per simulated second.
            if t % 1000 >= TICK_MS {
                return;
            }
            let secs = t / 1000;
            self.clock().set(START_CLOCK + secs);
            let n = patient.numerics();
            self.hr().set_if_changed(n.hr);
            self.spo2().set_if_changed(n.spo2);
            self.rr().set_if_changed(n.rr);
            self.temp().set_if_changed(n.temp);
            let paused = self.paused().get_untracked();
            if paused > 0 {
                self.paused().set(paused - 1);
            }

            // NIBP: on request or at the auto interval, a cuff cycle — inflate to 165 mmHg in
            // 3 s, deflate stepwise for 6 s, then the result.
            let every = self.nibp_every().get_untracked() * 60;
            let due = every > 0 && secs > 0 && secs % every == 0;
            if nibp_started.is_none() && (self.nibp_request().get_untracked() || due) {
                nibp_started = Some(secs);
            }
            self.nibp_request().set_if_changed(false);
            if let Some(s0) = nibp_started {
                let dt = (secs - s0) as i32;
                if dt < 9 {
                    let p = if dt < 3 { 40 + dt * 42 } else { 165 - (dt - 3) * 16 };
                    self.cuff().set(Some(p));
                } else {
                    self.cuff().set(None);
                    self.nibp().set(patient.measure_nibp());
                    self.nibp_at().set(START_CLOCK + secs);
                    nibp_started = None;
                }
            }

            self.evaluate_alarms(START_CLOCK + secs);

            if secs % TREND_STEP_S == 0 {
                let point = TrendPoint {
                    hr: n.hr.clamp(0, 255) as u8,
                    spo2: n.spo2.clamp(0, 100) as u8,
                    rr: n.rr.clamp(0, 255) as u8,
                    temp: n.temp.clamp(0, 999) as u16,
                    nibp: self.nibp().get_untracked(),
                };
                self.trends().with_mut(|tr| {
                    tr.points.push_back(point);
                    while tr.points.len() > TREND_CAPACITY {
                        tr.points.pop_front();
                    }
                });
                self.trend_rev().update(|r| *r += 1);
            }
        });
    }

    /// Raises, updates and clears alarm conditions against the limits (once per second).
    fn evaluate_alarms(self, now: u32) {
        let v = Vitals {
            hr: self.hr().get_untracked(),
            spo2: self.spo2().get_untracked(),
            rr: self.rr().get_untracked(),
            sys: self.nibp().get_untracked().sys,
        };
        let limits = self.limits().get_untracked();
        let before = self.alarms().get_untracked();
        let mut alarms: Vec<Alarm> = Vec::with_capacity(Condition::ALL.len());
        let mut raised = false;
        for cond in Condition::ALL {
            let Some((value, limit)) = cond.check(&v, &limits) else {
                continue;
            };
            let since = before.iter().find(|a| a.cond == cond).map_or_else(
                || {
                    raised = true;
                    now
                },
                |a| a.since,
            );
            alarms.push(Alarm {
                cond,
                value,
                limit,
                since,
            });
        }
        // Highest priority first, then the oldest.
        alarms.sort_by_key(|a| (core::cmp::Reverse(a.cond.priority()), a.since));
        let mut log = self.log().get_untracked();
        for a in &alarms {
            let worse = |w: i32| {
                if a.value > a.limit {
                    a.value.max(w)
                } else {
                    a.value.min(w)
                }
            };
            if let Some(e) = log.iter_mut().find(|e| e.cond == a.cond && e.end.is_none()) {
                e.worst = worse(e.worst);
            } else {
                let id = log.first().map_or(0, |e| e.id + 1);
                log.insert(
                    0,
                    LogEntry {
                        id,
                        cond: a.cond,
                        start: now,
                        end: None,
                        worst: a.value,
                    },
                );
            }
        }
        for e in log.iter_mut().filter(|e| e.end.is_none()) {
            if !alarms.iter().any(|a| a.cond == e.cond) {
                e.end = Some(now);
            }
        }
        log.truncate(LOG_CAPACITY);
        self.log().set_if_changed(log);
        if raised {
            // A new condition ends the audio pause: it must be heard.
            self.paused().set_if_changed(0);
        }
        self.alarms().set_if_changed(alarms);
    }

    /// The highest-priority active alarm, if any (the banner shows it).
    pub fn top_alarm(self) -> Option<Alarm> {
        self.alarms().with(|a| a.first().cloned())
    }

    /// The alarm condition of `param`, if one is active (its tile is highlighted).
    pub fn alarm_of(self, param: Param) -> Option<Priority> {
        self.alarms().with(|a| {
            a.iter()
                .find(|a| a.cond.param() == param)
                .map(|a| a.cond.priority())
        })
    }

    /// Pauses alarm audio for [`PAUSE_S`] (or resumes it when already paused).
    pub fn toggle_silence(self) {
        self.paused().update(|p| *p = if *p > 0 { 0 } else { PAUSE_S });
    }

    /// The UI's request for an NIBP measurement (read and cleared by the tick).
    fn nibp_request(self) -> Signal<bool> {
        self.0.with(|s| s.nibp_request)
    }

    /// Starts an NIBP measurement now.
    pub fn start_nibp(self) {
        self.nibp_request().set(true);
    }
}

/// Widens the column envelope `e` to include `v` (starting it with the first sample).
fn widen(e: &mut Option<Envelope>, v: i32) {
    match e {
        Some(e) => e.add(v),
        None => *e = Some(Envelope::of(v)),
    }
}

/// `hh:mm` of clock seconds.
pub fn hhmm(secs: u32) -> (u32, u32) {
    (secs / 3600 % 24, secs / 60 % 60)
}
