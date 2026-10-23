//! The simulated audio engine: what the visualizer and the level meters "hear".
//!
//! The player plays no real audio. Instead, [`listen`] computes, for any track and any
//! moment in it, the spectrum a real decoder + FFT would report: 32 analyzer bands (log
//! spaced, 32 Hz–16 kHz, in dB) and the left/right levels. It is a pure function of
//! `(track, time, equalizer)` — no state, no clock, no randomness beyond hashes of the track's
//! seed — so the same moment always looks the same, scrubbing works, and scripted runs are
//! reproducible.
//!
//! **The model.** A track is a little arrangement on a beat grid at its tempo. Each
//! instrument adds a bump of energy (a Gaussian in log-frequency) around its frequency,
//! shaped in time by an envelope locked to the beat:
//!
//! | Voice | Where | When |
//! |-------|-------|------|
//! | kick | ~55 Hz | every beat, sharp decay |
//! | bass | 40–80 Hz + 2nd harmonic | a note per half bar |
//! | snare | ~200 Hz body + 2.5 kHz noise | beats 2 and 4 |
//! | hats | ~9 kHz | every eighth note, open on the off-beat |
//! | pad | a triad, 200–600 Hz | a chord per bar, slowly swelling |
//! | lead | 500 Hz–2 kHz | a pentatonic note per eighth, in choruses |
//!
//! The genre's [`Profile`] sets how loud each voice is and the spectral tilt (rock is bright,
//! soul is warm). Eight-bar sections (intro, verse, chorus, breakdown) chosen from the
//! track's seed make the energy rise and fall like a song. The equalizer is applied last:
//! each band is moved by [`eq::response_db`] at its centre frequency, so boosting the bass
//! visibly lifts the left of the spectrum.
//!
//! All of this is a few hundred multiply-adds per frame — far below the cost of drawing it.

// Times and indices are far below 2^24, so their f32 conversions are exact.
#![allow(clippy::cast_precision_loss)]

use crate::catalog::{Genre, Track};
use crate::eq;
use crate::util::mix;

/// Number of analyzer bands.
pub const BANDS: usize = 32;
/// The lowest analyzer band's centre frequency.
const LOWEST_HZ: f32 = 32.0;
/// Octaves from the lowest to the highest band (32 Hz · 2⁹ = 16 kHz).
const SPAN_OCTAVES: f32 = 9.0;

/// The centre frequency of analyzer band `i`: equal ratios between neighbours, like the
/// ear hears pitch.
pub fn band_hz(i: usize) -> f32 {
    LOWEST_HZ * (SPAN_OCTAVES * i as f32 / (BANDS - 1) as f32).exp2()
}

/// What the meters and the visualizer get for one instant.
#[derive(Clone, Copy, Debug)]
pub struct Sound {
    /// Energy per band in dB relative to full scale (about −60 … 0).
    pub bands: [f32; BANDS],
    /// Left and right channel levels in dBFS.
    pub left: f32,
    pub right: f32,
}

impl Sound {
    /// Silence.
    pub const SILENT: Sound = Sound {
        bands: [-90.0; BANDS],
        left: -90.0,
        right: -90.0,
    };
}

/// How a genre sounds: voice levels (linear power) and spectral tilt.
#[derive(Clone, Copy, Debug)]
pub struct Profile {
    kick: f32,
    bass: f32,
    snare: f32,
    hats: f32,
    pad: f32,
    lead: f32,
    /// Spectral slope in dB per octave around 1 kHz (negative: warmer).
    tilt: f32,
    /// Kick on beats 1 and 3 only, snare on 3 (a half-time feel).
    half_time: bool,
}

/// The sound of `genre`.
pub fn profile(genre: Genre) -> Profile {
    let p = |kick, bass, snare, hats, pad, lead, tilt, half_time| Profile {
        kick,
        bass,
        snare,
        hats,
        pad,
        lead,
        tilt,
        half_time,
    };
    match genre {
        Genre::Synthwave => p(0.9, 0.7, 0.5, 0.25, 0.8, 0.7, -1.0, false),
        Genre::IndiePop => p(0.7, 0.6, 0.6, 0.35, 0.5, 0.8, 0.0, false),
        Genre::Jazz => p(0.2, 0.55, 0.15, 0.3, 0.6, 0.7, -1.5, false),
        Genre::Soul => p(0.6, 0.8, 0.4, 0.2, 0.6, 0.6, -2.0, true),
        Genre::Ambient => p(0.0, 0.3, 0.0, 0.05, 1.0, 0.4, -2.5, false),
        Genre::House => p(1.0, 0.8, 0.3, 0.5, 0.5, 0.4, -0.5, false),
        Genre::Folk => p(0.25, 0.4, 0.2, 0.15, 0.7, 0.8, -1.0, false),
        Genre::HipHop => p(1.0, 0.9, 0.6, 0.3, 0.4, 0.5, -1.5, true),
        Genre::Rock => p(0.8, 0.7, 0.8, 0.45, 0.6, 0.9, 1.0, false),
    }
}

/// The section a bar belongs to, which scales the voices.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Section {
    Intro,
    Verse,
    Chorus,
    Breakdown,
}

fn section(track: &Track, bar: u32) -> Section {
    if bar < 4 {
        return Section::Intro;
    }
    match (bar - 4) / 8 {
        // The first chorus always comes second; later sections follow the seed.
        0 => Section::Verse,
        1 => Section::Chorus,
        n => match mix(track.seed ^ u64::from(n)) % 5 {
            0 => Section::Breakdown,
            1 | 2 => Section::Verse,
            _ => Section::Chorus,
        },
    }
}

/// A hash of `(track, a, b)` as a number in `0.0..1.0`.
fn hash01(seed: u64, a: u32, b: u32) -> f32 {
    let h = mix(seed ^ (u64::from(a) << 32) ^ u64::from(b).wrapping_mul(0x9E37_79B9));
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Adds a voice to the band energies: `power` spread around `hz` as a Gaussian `width`
/// octaves wide (in log-frequency, so a voice is as wide at 8 kHz as at 80 Hz, as heard).
fn bump(energy: &mut [f32; BANDS], hz: f32, width: f32, power: f32) {
    if power <= 0.0 {
        return;
    }
    // Band positions in octaves are `i · SPAN/(BANDS−1)`; the voice sits at `log2(hz/LOWEST)`.
    let centre = (hz / LOWEST_HZ).log2();
    for (i, e) in energy.iter_mut().enumerate() {
        let d = (SPAN_OCTAVES * i as f32 / (BANDS - 1) as f32 - centre) / width;
        if d.abs() < 3.0 {
            *e += power * (-d * d).exp();
        }
    }
}

/// An exponential decay envelope: 1 at `phase = 0`, `e^-rate` at `phase = 1`.
fn decay(phase: f32, rate: f32) -> f32 {
    (-phase * rate).exp()
}

/// Minor-pentatonic steps for melodies and bass lines (semitones above the root).
const PENTATONIC: [f32; 5] = [0.0, 3.0, 5.0, 7.0, 10.0];

/// The pitch `steps` semitones above `root`.
fn note(root: f32, steps: f32) -> f32 {
    root * (steps / 12.0).exp2()
}

/// What the analyzer reports for `track` at `t_ms` from its start, with the equalizer's
/// band gains `eq_db` (all zero when it is off).
pub fn listen(track: &Track, genre: Genre, t_ms: u32, eq_db: &[f32; eq::BANDS]) -> Sound {
    let p = profile(genre);
    let seed = track.seed;
    let beats = t_ms as f32 * f32::from(track.bpm) / 60_000.0;
    let beat = beats as u32;
    let phase = beats.fract();
    let bar = beat / 4;
    let in_bar = beat % 4;
    let eighth = (beats * 2.0) as u32;
    let eighth_phase = (beats * 2.0).fract();

    let sec = section(track, bar);
    let (drums, lead_on, level) = match sec {
        Section::Intro => (0.0, false, 0.55),
        Section::Verse => (0.85, false, 0.8),
        Section::Chorus => (1.0, true, 1.0),
        Section::Breakdown => (0.0, true, 0.6),
    };

    // The key of the track and the chord of this bar (a i–VI–III–VII style loop).
    let root = 41.2 * (mix(seed) % 7) as f32 / 7.0 + 41.2; // 41–82 Hz
    let progression = [0.0, 8.0, 3.0, 10.0];
    let chord_root = note(root, progression[(bar % 4) as usize]);

    let mut core = [0.0f32; BANDS]; // centred voices
    let mut left = [0.0f32; BANDS]; // voices panned left
    let mut right = [0.0f32; BANDS]; // voices panned right

    // Kick: a thump on every beat (on 1 and 3 in half time).
    let kick_beat = !p.half_time || in_bar % 2 == 0;
    if kick_beat {
        let env = decay(phase, 9.0);
        bump(&mut core, 55.0, 0.55, p.kick * drums * env * 2.2);
        bump(&mut core, 3200.0, 0.8, p.kick * drums * env * 0.05); // the beater's click
    }
    // Bass: one pentatonic note per half bar, held with a soft decay.
    let half = beat / 2;
    let bass_note = note(chord_root, PENTATONIC[(mix(seed ^ u64::from(half)) % 3) as usize]);
    let bass_env = 0.55 + 0.45 * decay((beats / 2.0).fract(), 2.5);
    let bass_level = if sec == Section::Intro { 0.4 } else { 1.0 };
    bump(&mut core, bass_note, 0.35, p.bass * bass_env * bass_level);
    bump(
        &mut core,
        bass_note * 2.0,
        0.3,
        p.bass * bass_env * bass_level * 0.35,
    );
    // Snare: on the back beat (beat 3 in half time).
    let snare_beat = if p.half_time { in_bar == 2 } else { in_bar % 2 == 1 };
    if snare_beat {
        let env = decay(phase, 11.0);
        bump(&mut core, 210.0, 0.5, p.snare * drums * env * 0.9);
        bump(&mut core, 2600.0, 1.6, p.snare * drums * env * 0.45);
    }
    // Hats: every eighth, the off-beat ones open (longer), slightly right of centre.
    let open = eighth % 2 == 1;
    let hat_env = decay(eighth_phase, if open { 5.0 } else { 16.0 });
    let accent = 0.7 + 0.6 * hash01(seed, eighth, 7);
    let hats = p.hats * hat_env * accent * if sec == Section::Intro { 0.6 } else { 1.0 };
    bump(&mut right, 9000.0, 0.7, hats * 0.12);
    bump(&mut left, 9000.0, 0.7, hats * 0.06);
    bump(&mut right, 14000.0, 0.5, hats * 0.04);
    // Pad: the bar's triad two octaves up, swelling over four bars.
    let swell = 0.75 + 0.25 * (beats / 16.0 * std::f32::consts::TAU).sin();
    for (k, steps) in [0.0, 3.0, 7.0].into_iter().enumerate() {
        let hz = note(chord_root * 4.0, steps);
        let pad = p.pad * swell * 0.18;
        if k == 1 {
            bump(&mut left, hz, 0.25, pad);
        } else {
            bump(&mut core, hz, 0.25, pad);
        }
        bump(&mut right, hz * 2.0, 0.25, pad * 0.3);
    }
    // Lead: a pentatonic line in choruses (and breakdowns), a note per eighth.
    if lead_on {
        let step = PENTATONIC[(mix(seed ^ 0xA11CE ^ u64::from(eighth)) % 5) as usize];
        let octave = if hash01(seed, eighth, 3) > 0.6 { 24.0 } else { 12.0 };
        let hz = note(chord_root * 8.0, step + octave - 12.0);
        let env = 0.35 + 0.65 * decay(eighth_phase, 3.0);
        bump(&mut left, hz, 0.22, p.lead * env * 0.22);
        bump(&mut core, hz * 2.0, 0.22, p.lead * env * 0.06);
    }

    // Fade in over the first 1.5 s and out over the last 4 s, like a mastered track.
    let end_ms = track.secs * 1000;
    let fade_in = (t_ms as f32 / 1500.0).min(1.0);
    let fade_out = (end_ms.saturating_sub(t_ms) as f32 / 4000.0).min(1.0);
    let gain = level * fade_in * fade_out;

    let mut sound = Sound::SILENT;
    let (mut pl, mut pr) = (0.0f32, 0.0f32);
    for i in 0..BANDS {
        let hz = band_hz(i);
        // A quiet floor (room tone, reverb tails) so no band is ever exactly empty.
        let floor = 0.000_25 * (0.5 + hash01(seed, t_ms / 45, i as u32));
        // Spectral tilt around 1 kHz, then the equalizer.
        let shape_db = p.tilt * (hz / 1000.0).log2() + eq::response_db(eq_db, hz);
        let shape = 10f32.powf(shape_db / 10.0);
        // A little flicker so steady notes still look alive.
        let flicker = 0.8 + 0.4 * hash01(seed, t_ms / 33, i as u32 + 64);
        let l = (core[i] + left[i] + floor) * shape * gain * flicker;
        let r = (core[i] + right[i] + floor) * shape * gain * flicker;
        sound.bands[i] = 10.0 * (l.midpoint(r) + 1e-9).log10();
        pl += l;
        pr += r;
    }
    // Levels: the total power of each channel, scaled so a loud chorus peaks near −3 dBFS.
    sound.left = 10.0 * (pl * 0.12 + 1e-9).log10();
    sound.right = 10.0 * (pr * 0.12 + 1e-9).log10();
    sound
}
