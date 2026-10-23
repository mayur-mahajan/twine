//! The player: what is playing, what plays next, and the frame clock that drives playback,
//! the simulated audio and the visualizer.
//!
//! **Store.** [`Player`] is a `Copy` struct of signals (Twine's store pattern): every page
//! reads the parts it shows and calls its methods to act. Each signal is fine-grained on
//! purpose — the seek bar binds `position`, the play button `playing`, the queue `queue` —
//! so a position update every frame re-runs the seek bar's bindings and nothing else.
//!
//! **Queue.** Playing from a list (songs, an album, a playlist) makes that list the
//! *context*; the tracks after the chosen one become the up-next queue (shuffled when shuffle
//! is on, with a seeded generator). Each queue entry has its own key, so the same song can be
//! queued twice and the keyed queue view moves rows instead of rebuilding them. Previous
//! tracks go to a history stack for the ⏮ button.
//!
//! **Frame clock.** [`Player::run`] installs one 16 ms timer (60 fps). Each tick reads the
//! engine's clock (the simulator's deterministic clock in headless runs), advances the
//! position by the real elapsed time, eases the equalizer, asks the simulated audio engine
//! what it hears and runs the visualizer's ballistics. When paused and everything has come
//! to rest, the tick writes nothing, so nothing redraws and the UI idles.

// Volume in dB from a percentage.
#![allow(clippy::cast_precision_loss)]

use twine::prelude::*;
use twine::view::EngineAccess;

use crate::audio::{self, Sound};
use crate::catalog::catalog;
use crate::eq::Equalizer;
use crate::util::Rng;
use crate::viz::{Analyzer, Levels, Spectrum};

/// The repeat mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    Off,
    /// Start the context again when it ends.
    All,
    /// Play the current track again.
    One,
}

/// One queued track. `key` identifies the entry (a track may be queued more than once).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: u32,
    pub track: u32,
}

/// Seeking with ⏮ restarts the track when more than this much of it has played.
const RESTART_MS: u32 = 3000;

/// The player's state.
#[derive(Clone, Copy)]
pub struct Player {
    /// The loaded track.
    pub current: Signal<u32>,
    pub playing: Signal<bool>,
    /// Playback position in milliseconds.
    pub position: Signal<u32>,
    /// Volume, 0–100.
    pub volume: Signal<i32>,
    pub muted: Signal<bool>,
    pub shuffle: Signal<bool>,
    pub repeat: Signal<Repeat>,
    /// Up next.
    pub queue: Signal<Vec<Entry>>,
    /// The visualizer's frame (written by the frame clock).
    pub spectrum: Signal<Spectrum>,
    /// The level meters (written by the frame clock).
    pub levels: Signal<Levels>,
    /// Where playback started: the list repeat-all replays and un-shuffle restores.
    context: Signal<Vec<u32>>,
    history: Signal<Vec<u32>>,
    next_key: Signal<u32>,
    shuffles: Signal<u64>,
}

impl Player {
    /// A player with `track` loaded from the list `context`, paused at `position_ms`.
    pub fn new(cx: Scope, context: Vec<u32>, track: u32, position_ms: u32) -> Self {
        let p = Self {
            current: cx.signal(track),
            playing: cx.signal(false),
            position: cx.signal(position_ms),
            volume: cx.signal(72),
            muted: cx.signal(false),
            shuffle: cx.signal(false),
            repeat: cx.signal(Repeat::All),
            queue: cx.signal(Vec::new()),
            spectrum: cx.signal(Spectrum::default()),
            levels: cx.signal(Levels::default()),
            context: cx.signal(Vec::new()),
            history: cx.signal(Vec::new()),
            next_key: cx.signal(0),
            shuffles: cx.signal(0),
        };
        let at = context.iter().position(|&t| t == track).unwrap_or(0);
        p.load_context(context, at);
        p
    }

    /// Plays `list[index]` and queues the rest of `list` after it.
    pub fn play_list(self, list: &[u32], index: usize) {
        let Some(&track) = list.get(index) else {
            return;
        };
        self.push_history();
        self.load_context(list.to_vec(), index);
        self.start(track);
    }

    /// Makes `list` the context with `list[index]` current, and queues what follows it.
    fn load_context(self, list: Vec<u32>, index: usize) {
        let rest: Vec<u32> = list.iter().skip(index + 1).copied().collect();
        self.context.set(list);
        self.set_queue(rest);
    }

    /// Replaces the queue with `tracks` (shuffled when shuffle is on), with fresh keys.
    fn set_queue(self, mut tracks: Vec<u32>) {
        if self.shuffle.get_untracked() {
            self.shuffle_in_place(&mut tracks);
        }
        let first = self.next_key.get_untracked();
        self.next_key.set(first + tracks.len() as u32);
        self.queue.set(
            tracks
                .into_iter()
                .zip(first..)
                .map(|(track, key)| Entry { key, track })
                .collect(),
        );
    }

    /// A Fisher–Yates shuffle from a seeded generator: a different order on every shuffle,
    /// the same sequence of orders on every run.
    fn shuffle_in_place(self, tracks: &mut [u32]) {
        let n = self.shuffles.get_untracked();
        self.shuffles.set(n + 1);
        let mut rng = Rng::new(0x5_4FF1E ^ n.wrapping_mul(0x9E37_79B9));
        for i in (1..tracks.len()).rev() {
            tracks.swap(i, rng.range(0, i as u32) as usize);
        }
    }

    /// Loads `track` at its start and plays it.
    fn start(self, track: u32) {
        self.current.set(track);
        self.position.set(0);
        self.playing.set(true);
    }

    fn push_history(self) {
        let cur = self.current.get_untracked();
        self.history.update(|h| h.push(cur));
    }

    /// Play / pause.
    pub fn toggle(self) {
        self.playing.update(|p| *p = !*p);
    }

    /// The next track: the head of the queue; at the end, the context again (repeat all) or
    /// stop at the start of the last track.
    pub fn next(self) {
        if self.queue.with_untracked(Vec::is_empty) {
            if self.repeat.get_untracked() == Repeat::Off {
                self.playing.set(false);
                self.position.set(0);
                return;
            }
            self.set_queue(self.context.get_untracked());
        }
        let head = self.queue.with_untracked(|q| q.first().copied());
        if let Some(e) = head {
            self.push_history();
            self.queue.update(|q| {
                q.remove(0);
            });
            self.start(e.track);
        }
    }

    /// The previous track, or the start of this one once it has played a few seconds.
    pub fn prev(self) {
        let prev = self.history.with_untracked(|h| h.last().copied());
        match prev {
            Some(track) if self.position.get_untracked() < RESTART_MS => {
                self.history.update(|h| {
                    h.pop();
                });
                // The current track goes back to the front of the queue.
                let cur = self.current.get_untracked();
                let key = self.next_key.get_untracked();
                self.next_key.set(key + 1);
                self.queue.update(|q| q.insert(0, Entry { key, track: cur }));
                self.start(track);
            }
            _ => self.position.set(0),
        }
    }

    /// Jumps to `ms` in the current track.
    pub fn seek(self, ms: u32) {
        let len = catalog().track(self.current.get_untracked()).secs * 1000;
        self.position.set(ms.min(len.saturating_sub(1)));
    }

    /// Seeks by `delta_ms` (negative: back).
    pub fn seek_by(self, delta_ms: i32) {
        let pos = self.position.get_untracked().saturating_add_signed(delta_ms);
        self.seek(pos);
    }

    /// Changes the volume by `delta` percent (and unmutes).
    pub fn nudge_volume(self, delta: i32) {
        self.muted.set(false);
        self.volume.update(|v| *v = (*v + delta).clamp(0, 100));
    }

    /// Shuffle on: the queue is shuffled. Off: the queue is the context's order again, from
    /// the current track on.
    pub fn toggle_shuffle(self) {
        let on = !self.shuffle.get_untracked();
        self.shuffle.set(on);
        if on {
            let mut tracks: Vec<u32> = self.queue.with_untracked(|q| q.iter().map(|e| e.track).collect());
            self.shuffle_in_place(&mut tracks);
            self.queue.update(|q| {
                for (e, t) in q.iter_mut().zip(tracks) {
                    e.track = t;
                }
            });
        } else {
            let cur = self.current.get_untracked();
            let list = self.context.get_untracked();
            let at = list.iter().position(|&t| t == cur).map_or(list.len(), |i| i + 1);
            self.set_queue(list[at..].to_vec());
        }
    }

    /// Off → all → one → off.
    pub fn cycle_repeat(self) {
        self.repeat.update(|r| {
            *r = match r {
                Repeat::Off => Repeat::All,
                Repeat::All => Repeat::One,
                Repeat::One => Repeat::Off,
            }
        });
    }

    /// Removes a queue entry.
    pub fn remove(self, key: u32) {
        self.queue.update(|q| q.retain(|e| e.key != key));
    }

    /// Moves a queue entry `by` places (negative: earlier), within the queue.
    pub fn shift(self, key: u32, by: i32) {
        self.queue.update(|q| {
            if let Some(i) = q.iter().position(|e| e.key == key) {
                let j = (i as i32 + by).clamp(0, q.len() as i32 - 1) as usize;
                let e = q.remove(i);
                q.insert(j, e);
            }
        });
    }

    /// Plays a queue entry now, skipping the entries before it.
    pub fn jump_to(self, key: u32) {
        let Some(i) = self.queue.with_untracked(|q| q.iter().position(|e| e.key == key)) else {
            return;
        };
        let track = self.queue.with_untracked(|q| q[i].track);
        self.push_history();
        self.queue.update(|q| {
            q.drain(..=i);
        });
        self.start(track);
    }

    /// Empties the queue.
    pub fn clear_queue(self) {
        self.queue.set(Vec::new());
    }

    /// The track ran out: repeat it, or go on.
    fn ended(self) {
        if self.repeat.get_untracked() == Repeat::One {
            self.position.set(0);
        } else {
            self.next();
        }
    }

    /// Installs the frame clock (see the module docs) in `cx`.
    pub fn run(self, cx: Scope, eq: Equalizer) {
        let rt = cx.runtime();
        let mut last: Option<Instant> = None;
        let mut analyzer = Analyzer::default();
        cx.interval(Duration::ms(16), move || {
            let Some(now) = EngineAccess::with(rt, |e| e.now()) else {
                return;
            };
            // Real elapsed time (a slow frame advances the song by as much as it took).
            let dt = last
                .map_or(16, |l| now.saturating_duration_since(l).as_millis_u32())
                .min(100);
            last = Some(now);
            eq.advance(dt);

            let playing = self.playing.get_untracked();
            let track = catalog().track(self.current.get_untracked());
            let mut pos = self.position.get_untracked();
            if playing {
                pos += dt;
                if pos >= track.secs * 1000 {
                    self.ended();
                    return;
                }
                self.position.set(pos);
            } else if analyzer.at_rest() {
                return; // paused and settled: nothing to draw
            }
            let sound = if playing {
                audio::listen(track, catalog().genre_of(track.id), pos, &eq.gains_db())
            } else {
                Sound::SILENT
            };
            let volume = if self.muted.get_untracked() {
                0
            } else {
                self.volume.get_untracked()
            };
            let volume_db = if volume == 0 {
                -96.0
            } else {
                40.0 * (volume as f32 / 100.0).log10()
            };
            let (spectrum, levels) = analyzer.step(&sound, volume_db, playing, dt);
            if self.spectrum.get_untracked() != spectrum {
                self.spectrum.set(spectrum);
            }
            if self.levels.get_untracked() != levels {
                self.levels.set(levels);
            }
        });
    }
}
