//! The patient simulator: a deterministic stand-in for the monitor's acquisition front end
//! (ECG amplifier, pulse oximeter, impedance pneumograph, cuff and temperature probe).
//!
//! On a real monitor these samples arrive from an ADC at a fixed rate; here [`Patient::step`]
//! synthesises them, 250 times per simulated second, from a small physiological model. The UI
//! never knows the difference: it reads samples and numerics exactly as it would read them from
//! a sensor task.
//!
//! **Deterministic.** Everything is a function of the seed and of the number of steps taken —
//! no wall clock, no floating point, no OS randomness — so a scripted headless run (and the
//! README GIF made from one) is reproducible bit for bit.
//!
//! **The model**, in integers throughout (angles in 0.1°, [`cos`] returns `cos · 32767`):
//!
//! - *ECG (lead II):* each beat is a sum of raised-cosine bumps — P, Q, R, S and T waves — placed
//!   relative to the R peak, so the QRS keeps its width at any rate while the QT interval
//!   shortens as the heart speeds up. Beat-to-beat intervals carry heart-rate variability and
//!   respiratory sinus arrhythmia, the baseline wanders with breathing, and a little noise
//!   rides on top.
//! - *SpO2 plethysmogram:* the pulse arrives ~200 ms after the R wave: a fast systolic upstroke,
//!   a slower decay and a dicrotic notch. The two most recent beats are summed so a fast heart
//!   rate overlaps them smoothly. Its amplitude follows peripheral perfusion.
//! - *Respiration:* inspiration over ~38 % of the cycle, a slower expiration.
//! - *Numerics:* computed like a monitor does — heart rate from the mean of the last R-R
//!   intervals, SpO2 smoothed over a few seconds — and published once per second.
//!
//! **The scenario** repeats every [`SCENARIO_MS`]: a stable patient, then a desaturation
//! (SpO2 falls to ~85 %, the heart and breathing rates climb), a plateau, and recovery. It is
//! what makes the alarm system visible in the demo.

use twine::core::Angle;
use twine::core::math::cos;

/// Model steps per simulated second (one step = 4 ms).
pub const RATE_HZ: u32 = 250;
/// Length of one step, in ms.
pub const STEP_MS: u32 = 1000 / RATE_HZ;
/// The scenario loops after this long (ms of simulated time).
pub const SCENARIO_MS: u32 = 180_000;

/// A small, fast, seedable pseudo-random generator (xorshift32): the simulator's only source
/// of variation, so the same seed always gives the same patient.
#[derive(Clone, Debug)]
pub struct Rng(u32);

impl Rng {
    /// A generator from `seed` (0 is replaced: xorshift has no zero state).
    pub const fn new(seed: u32) -> Self {
        Self(if seed == 0 { 0x9E37_79B9 } else { seed })
    }

    /// The next 32 random bits.
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// A value in `-amp..=amp`.
    pub fn jitter(&mut self, amp: i32) -> i32 {
        let span = (2 * amp + 1).max(1) as u32;
        (self.next_u32() % span) as i32 - amp
    }
}

/// `amp · (1 + cos(π · d / hw)) / 2` for `|d| < hw`, else 0: a smooth bump of half-width `hw`
/// centred on `d = 0`. The building block of every waveform.
fn bump(d: i32, hw: i32, amp: i32) -> i32 {
    if hw <= 0 || d <= -hw || d >= hw {
        return 0;
    }
    let c = cos(Angle::deci_deg(1800 * d / hw));
    amp * (32_767 + c) / 65_534
}

/// A bump with different half-widths before and after its centre (a fast rise, a slow fall).
fn skewed_bump(d: i32, rise: i32, fall: i32, amp: i32) -> i32 {
    if d < 0 {
        bump(d, rise, amp)
    } else {
        bump(d, fall, amp)
    }
}

/// `p` (0..=1000) eased with smoothstep, `3p² − 2p³`, still in 0..=1000.
fn smoothstep(p: i32) -> i32 {
    let p = p.clamp(0, 1000);
    p * p * (3000 - 2 * p) / 1_000_000
}

/// Moves `from` towards `to` as `t` goes from `t0` to `t1` (ms), eased.
fn ramp(t: u32, t0: u32, t1: u32, from: i32, to: i32) -> i32 {
    if t <= t0 {
        return from;
    }
    if t >= t1 {
        return to;
    }
    let p = ((t - t0) * 1000 / (t1 - t0)) as i32;
    from + (to - from) * smoothstep(p) / 1000
}

/// What the scenario asks of the patient at a moment, in tenths (72.0 bpm = 720).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Targets {
    hr: i32,
    spo2: i32,
    rr: i32,
    /// Perfusion of the finger (pleth amplitude), per mille.
    perfusion: i32,
}

/// The scripted course of the patient (`t`: ms since the scenario started).
///
/// | Time | Course |
/// |------|--------|
/// | 0–3 s | stable: HR 74, SpO2 97 %, RR 16 |
/// | 3–10 s | desaturation: SpO2 → 85 %, HR → 104, RR → 27, perfusion drops |
/// | 10–30 s | plateau |
/// | 30–42 s | recovery to SpO2 96 %, HR 78, RR 17 |
/// | 42–180 s | stable, then the cycle repeats |
fn scenario(t: u32) -> Targets {
    let t = t % SCENARIO_MS;
    let (down0, down1, up0, up1) = (3_000, 10_000, 30_000, 42_000);
    let leg = |calm: i32, crisis: i32, after: i32| {
        if t < up0 {
            ramp(t, down0, down1, calm, crisis)
        } else {
            ramp(t, up0, up1, crisis, after)
        }
    };
    Targets {
        hr: leg(740, 1040, 780),
        spo2: leg(972, 848, 962),
        rr: leg(160, 270, 170),
        perfusion: leg(1000, 620, 920),
    }
}

/// One sample of each waveform (one model step).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sample {
    /// ECG lead II, in µV (an R wave is ~1.1 mV).
    pub ecg: i32,
    /// Plethysmogram, 0..=~1000 (arbitrary units, scaled by perfusion).
    pub pleth: i32,
    /// Respiration (thoracic impedance), about −500..=500.
    pub resp: i32,
    /// An R wave peaked in this step (the heart-beat indicator flashes).
    pub beat: bool,
}

/// The numerics a monitor displays, published once per second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Numerics {
    /// Heart rate, bpm.
    pub hr: i32,
    /// Oxygen saturation, %.
    pub spo2: i32,
    /// Respiration rate, breaths per minute.
    pub rr: i32,
    /// Core temperature, tenths of °C.
    pub temp: i32,
}

/// One non-invasive blood pressure measurement, mmHg.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Nibp {
    /// Systolic.
    pub sys: i32,
    /// Diastolic.
    pub dia: i32,
}

impl Nibp {
    /// Mean arterial pressure, the usual `dia + (sys − dia) / 3` estimate.
    pub const fn map(self) -> i32 {
        self.dia + (self.sys - self.dia) / 3
    }
}

/// How many R-R intervals the displayed heart rate averages (like a monitor's 4-beat average).
const RR_AVG: usize = 4;

/// The simulated patient: call [`step`](Self::step) [`RATE_HZ`] times per simulated second.
#[derive(Clone, Debug)]
pub struct Patient {
    rng: Rng,
    /// Simulated time, ms.
    t: u32,
    /// Time since the last R wave, ms.
    since_r: i32,
    /// The current and the previous R-R interval, ms.
    rr_ms: i32,
    prev_rr_ms: i32,
    /// The last R-R intervals, for the heart-rate numeric.
    rr_hist: [i32; RR_AVG],
    rr_next: usize,
    /// Respiratory phase, 0..100_000 per breath.
    breath: i32,
    /// SpO2 shown, in 1/10000 % (a slow exponential average of the scenario; the fine unit
    /// keeps the integer average from stalling short of its goal).
    spo2_avg: i32,
    /// Temperature, tenths of °C (a very slow drift).
    temp: i32,
}

impl Patient {
    /// A patient seeded with `seed`, settled at the start of the scenario.
    pub fn new(seed: u32) -> Self {
        let first = scenario(0);
        Self {
            rng: Rng::new(seed),
            t: 0,
            since_r: 0,
            rr_ms: 600_000 / first.hr,
            prev_rr_ms: 600_000 / first.hr,
            rr_hist: [600_000 / first.hr; RR_AVG],
            rr_next: 0,
            breath: 0,
            spo2_avg: first.spo2 * 1000,
            temp: 368,
        }
    }

    /// Simulated time since start, ms.
    pub fn time_ms(&self) -> u32 {
        self.t
    }

    /// Advances the model by one step ([`STEP_MS`]) and returns the new samples.
    pub fn step(&mut self) -> Sample {
        self.t = self.t.wrapping_add(STEP_MS);
        let target = scenario(self.t);
        let resp_phase = self.breath; // 0..100_000

        // Respiration: advance the phase at the scenario's rate (tenths of breaths/min).
        self.breath += target.rr * STEP_MS as i32 * 100_000 / 600_000;
        self.breath %= 100_000;
        let resp = respiration(resp_phase);

        // Cardiac timing: a new beat when the interval has elapsed. The next interval is the
        // scenario's, plus respiratory sinus arrhythmia (faster on inspiration) and HRV.
        self.since_r += STEP_MS as i32;
        let mut beat = false;
        if self.since_r >= self.rr_ms {
            self.since_r -= self.rr_ms;
            self.prev_rr_ms = self.rr_ms;
            self.rr_hist[self.rr_next] = self.rr_ms;
            self.rr_next = (self.rr_next + 1) % RR_AVG;
            let rsa = resp / 40; // ±12 ms
            self.rr_ms = (600_000 / target.hr.max(300) - rsa + self.rng.jitter(9)).max(250);
            beat = true;
        }

        let ecg = self.ecg(resp) + self.rng.jitter(9);
        let pleth = self.pleth() * target.perfusion / 1000;

        // SpO2 numeric: the oximeter averages over a few seconds (τ ≈ 2 s).
        let goal = target.spo2 * 1000;
        self.spo2_avg += (goal - self.spo2_avg) / 500;
        if self.t % 60_000 == 0 {
            self.temp = (self.temp + self.rng.jitter(1)).clamp(364, 372);
        }
        Sample {
            ecg,
            pleth,
            resp,
            beat,
        }
    }

    /// Lead II at the current moment: P, Q, R, S, T around the last R wave (and the P and Q
    /// of the next one), on a baseline that wanders with breathing.
    fn ecg(&self, resp: i32) -> i32 {
        let tr = self.since_r; // after the last R
        let tn = self.rr_ms - self.since_r; // before the next R
        // QT shortens with rate (Bazett-like): T peaks ~300 ms after R at 75 bpm.
        let t_peak = 170 + self.rr_ms / 6;
        let wander = resp / 12;
        wander
            + bump(tn - 165, 48, 120) // P wave of the next beat
            + bump(tn - 28, 14, -140) // Q
            + bump(tr, 24, 1100) + bump(tn, 24, 1100) // R (rising before, falling after)
            + bump(tr - 30, 16, -260) // S
            + skewed_bump(tr - t_peak, 90, 70, 290) // T wave
    }

    /// The plethysmogram: the pulses of the last two beats, each ~200 ms after its R wave.
    fn pleth(&self) -> i32 {
        let pulse = |t: i32| skewed_bump(t - 130, 120, 330, 1000) + bump(t - 400, 95, 230);
        let t = self.since_r - 200;
        pulse(t) + pulse(t + self.prev_rr_ms)
    }

    /// The numerics a monitor would show now.
    pub fn numerics(&self) -> Numerics {
        let mean_rr = self.rr_hist.iter().sum::<i32>() / RR_AVG as i32;
        Numerics {
            hr: (60_000 + mean_rr / 2) / mean_rr.max(1),
            spo2: (self.spo2_avg + 5000) / 10_000,
            rr: (scenario(self.t).rr + 5) / 10,
            temp: self.temp,
        }
    }

    /// A cuff measurement taken now: pressure rises a little with heart rate.
    pub fn measure_nibp(&mut self) -> Nibp {
        let hr = self.numerics().hr;
        Nibp {
            sys: 112 + (hr - 72) / 3 + self.rng.jitter(3),
            dia: 74 + (hr - 72) / 6 + self.rng.jitter(2),
        }
    }
}

/// The impedance pneumogram over one breath (`phase` 0..100_000): inspiration over the first
/// 38 %, then a slower expiration and a short pause.
fn respiration(phase: i32) -> i32 {
    const INSP: i32 = 38_000;
    let level = if phase < INSP {
        // 0 → 1000 along half a cosine.
        500 - cos(Angle::deci_deg(1800 * phase / INSP)) * 500 / 32_767
    } else {
        let p = (phase - INSP) * 1000 / (100_000 - INSP);
        // 1000 → 0, finishing early (end-expiratory pause).
        let p = (p * 13 / 10).min(1000);
        500 + cos(Angle::deci_deg(1800 * p / 1000)) * 500 / 32_767
    };
    level - 500
}

/// One point of the trend history (every [`TREND_STEP_S`] seconds).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrendPoint {
    /// Heart rate, bpm.
    pub hr: u8,
    /// SpO2, %.
    pub spo2: u8,
    /// Respiration rate, /min.
    pub rr: u8,
    /// Temperature, tenths of °C.
    pub temp: u16,
    /// The latest NIBP at this time.
    pub nibp: Nibp,
}

/// Seconds between two trend points.
pub const TREND_STEP_S: u32 = 10;
/// Trend points in the longest view (12 h).
pub const TREND_CAPACITY: usize = (12 * 3600 / TREND_STEP_S) as usize;

/// The last 12 hours before the demo starts, so the trend screen is populated from the first
/// frame: a slowly varying patient with two earlier, milder desaturations (one ~3 h ago, one
/// ~8 h ago) and an NIBP every 15 minutes. Oldest first.
pub fn trend_history(seed: u32) -> Vec<TrendPoint> {
    let mut rng = Rng::new(seed ^ 0x5EED_7E1D);
    let n = TREND_CAPACITY as i32;
    let mut walk = 0i32;
    let mut nibp = Nibp { sys: 121, dia: 77 };
    (0..n)
        .map(|i| {
            let ago_min = (n - i) * TREND_STEP_S as i32 / 60;
            // A slow circadian-ish drift plus a bounded random walk.
            walk = (walk + rng.jitter(2)).clamp(-60, 60);
            let slow = cos(Angle::deci_deg(i * 3600 / (n / 2))) * 4 / 32_767;
            let dip = |center: i32, depth: i32, width: i32| bump(ago_min - center, width, depth);
            let desat = dip(190, 8, 14) + dip(480, 5, 10);
            let hr = 76 + slow + walk / 10 + desat * 3 + rng.jitter(1);
            // Mostly steady, an occasional one-point dip (the numeric is an integer %).
            let spo2 = 97 - desat - i32::from(rng.next_u32() % 8 == 0);
            if i % (15 * 60 / TREND_STEP_S as i32) == 0 {
                nibp = Nibp {
                    sys: 118 + slow * 2 + rng.jitter(5),
                    dia: 76 + slow + rng.jitter(3),
                };
            }
            TrendPoint {
                hr: hr.clamp(30, 250) as u8,
                spo2: spo2.clamp(50, 100) as u8,
                rr: (15 + desat + rng.jitter(1)).clamp(4, 60) as u8,
                temp: (367 + slow / 2 + rng.jitter(1)) as u16,
                nibp,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_patient() {
        let (mut a, mut b) = (Patient::new(7), Patient::new(7));
        for _ in 0..5_000 {
            assert_eq!(a.step(), b.step());
        }
        assert_eq!(trend_history(7), trend_history(7));
    }

    #[test]
    fn scenario_desaturates_and_recovers() {
        let mut p = Patient::new(1);
        let mut at = |s: u32| {
            while p.time_ms() < s * 1000 {
                p.step();
            }
            p.numerics()
        };
        assert!(at(2).spo2 >= 96);
        let low = at(20);
        assert!(low.spo2 <= 87, "{low:?}");
        assert!(low.hr > 95, "{low:?}");
        assert!(at(60).spo2 >= 95);
    }
}
