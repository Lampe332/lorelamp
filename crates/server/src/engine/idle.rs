//! Unloading the model to give the graphics memory back: by hand ("Modell entladen") or after a
//! quiet time (setting `idle_unload_min`). While unloaded the supervisor runs no model server; the
//! next message (or "Modell laden") starts it again. Pure state, no I/O: the hub owns one and the
//! supervisor asks it through `Desired::unloaded`.

use std::time::{Duration, Instant};

/// The choices of the setting "Modell nach … entladen", in minutes (0 = off, the default).
pub const IDLE_CHOICES: [u32; 5] = [0, 5, 15, 30, 60];

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Idle {
    /// Wanted: no model server until someone sends or presses "load".
    pub unloaded: bool,
    /// The model is on its way up (a load was asked for, or the supervisor started it) and has not
    /// answered as ready yet: a message sent now waits for it instead of failing.
    pub waking: bool,
    /// Since when nothing happened: the last finished reply or the moment the model became ready.
    /// None: nothing is loaded that could be unloaded.
    pub since: Option<Instant>,
}

impl Idle {
    /// Unload now (the caller made sure no reply is being written).
    pub fn unload(&mut self) {
        *self = Idle { unloaded: true, waking: false, since: None };
    }

    /// Load again. Answers whether it was unloaded (only then does anything change).
    pub fn load(&mut self) -> bool {
        if !self.unloaded {
            return false;
        }
        self.unloaded = false;
        self.waking = true;
        true
    }

    /// A model server event from the supervisor (`starting`, `ready`, `failed`, …).
    pub fn event(&mut self, event: &str, now: Instant) {
        match event {
            "starting" | "switching" => self.waking = !self.unloaded,
            "ready" => {
                self.waking = false;
                self.since = Some(now);
            }
            // A crash is restarted by the supervisor: still on its way up, but nothing to unload.
            "crashed" => self.since = None,
            "failed" => {
                self.waking = false;
                self.since = None;
            }
            _ => {}
        }
    }

    /// A reply was finished (or stopped): the quiet time starts again.
    pub fn reply_done(&mut self, now: Instant) {
        if self.since.is_some() {
            self.since = Some(now);
        }
    }

    /// The quiet time `limit` is over: unload now. Never while a reply is written (`busy`), never
    /// with the setting off (`None`), never when nothing is loaded.
    pub fn due(&self, limit: Option<Duration>, busy: bool, now: Instant) -> bool {
        !self.unloaded && !busy && matches!((limit, self.since), (Some(limit), Some(since)) if now.duration_since(since) >= limit)
    }
}

/// The setting as a duration: `minutes` × `minute` (60 s; a test server may shorten the minute).
/// None: off.
pub fn limit(minutes: u32, minute: Duration) -> Option<Duration> {
    (minutes > 0).then(|| minute * minutes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unload_and_load_again() {
        let now = Instant::now();
        let mut idle = Idle::default();
        idle.event("starting", now);
        assert!(idle.waking, "a message sent while the model starts waits for it");
        idle.event("ready", now);
        assert_eq!(idle, Idle { unloaded: false, waking: false, since: Some(now) });
        assert!(!idle.load(), "loading a loaded model changes nothing");
        idle.unload();
        assert!(idle.unloaded && idle.since.is_none());
        assert!(idle.load(), "sending (or the button) loads it again");
        assert!(!idle.unloaded && idle.waking);
        idle.event("failed", now);
        assert!(!idle.waking, "a model that does not come up ends the wait");
    }

    #[test]
    fn the_quiet_time() {
        let t0 = Instant::now();
        let min = Duration::from_secs(60);
        let after = |m: u64| t0 + Duration::from_secs(60 * m);
        let mut idle = Idle::default();
        idle.event("ready", t0);
        assert!(!idle.due(None, false, after(600)), "off by default");
        assert!(!idle.due(limit(5, min), false, after(4)));
        assert!(idle.due(limit(5, min), false, after(5)), "counts from the moment it became ready");
        assert!(!idle.due(limit(5, min), true, after(50)), "never while a reply is written");
        idle.reply_done(after(4));
        assert!(!idle.due(limit(5, min), false, after(8)), "a finished reply starts the quiet time again");
        assert!(idle.due(limit(5, min), false, after(9)));
        idle.unload();
        assert!(!idle.due(limit(5, min), false, after(100)), "unloaded: nothing left to unload");
        idle.reply_done(after(101));
        assert_eq!(idle.since, None, "a reply finished without a loaded model does not start a quiet time");
        idle.event("crashed", t0);
        assert!(!idle.due(limit(5, min), false, after(200)));
        assert_eq!(limit(0, min), None);
        assert_eq!(limit(15, Duration::from_secs(1)), Some(Duration::from_secs(15)), "a test server's shorter minute");
    }

    #[test]
    fn starting_while_unloaded_is_not_waking() {
        let mut idle = Idle::default();
        idle.unload();
        idle.event("starting", Instant::now());
        assert!(!idle.waking);
    }
}
