//! Clock tick and ClockSpec.
//!
//! Watchman's clock is `c:<root_number>:<ticks>:<start>:<pid>`. Clients
//! treat it as opaque, but it has to be monotonic and has to change as
//! soon as any change is observed in the root.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic tick counter for a single root.
#[derive(Debug, Default)]
pub struct Clock {
    ticks: AtomicU64,
    root_number: u64,
    start_time: u64,
    pid: u32,
}

impl Clock {
    pub fn new(root_number: u64) -> Self {
        let start_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        Self {
            ticks: AtomicU64::new(1),
            root_number,
            start_time,
            pid: std::process::id(),
        }
    }

    pub fn current_tick(&self) -> u64 {
        self.ticks.load(Ordering::Acquire)
    }

    /// Increment and return the new tick.
    pub fn bump(&self) -> u64 {
        self.ticks.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub fn encode(&self, tick: u64) -> String {
        let mut s = String::with_capacity(32);
        write!(
            s,
            "c:{}:{}:{}:{}",
            self.start_time, self.pid, self.root_number, tick
        )
        .unwrap();
        s
    }

    pub fn current_string(&self) -> String {
        self.encode(self.current_tick())
    }
}

/// Parsed clock spec.  Unknown forms fall through to `Epoch(0)` so
/// `since` returns every file rather than erroring.
#[derive(Debug, Clone)]
pub enum ClockSpec {
    Epoch,
    Tick(u64),
    Named(String),
    /// Full `c:start:pid:root:tick` string.
    Full {
        start_time: u64,
        pid: u32,
        root_number: u64,
        tick: u64,
    },
    /// `scm:<vcs>:<mergebase>` — file-filter resolved via git/hg diff.
    Scm {
        vcs: ScmVcs,
        mergebase: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScmVcs {
    Git,
    Hg,
}

impl ClockSpec {
    pub fn parse(s: &str) -> Self {
        if let Some(rest) = s.strip_prefix("c:") {
            let parts: Vec<&str> = rest.split(':').collect();
            if parts.len() == 4 {
                if let (Ok(start_time), Ok(pid), Ok(root_number), Ok(tick)) = (
                    parts[0].parse::<u64>(),
                    parts[1].parse::<u32>(),
                    parts[2].parse::<u64>(),
                    parts[3].parse::<u64>(),
                ) {
                    return ClockSpec::Full {
                        start_time,
                        pid,
                        root_number,
                        tick,
                    };
                }
            }
            return ClockSpec::Epoch;
        }
        if let Some(name) = s.strip_prefix("n:") {
            return ClockSpec::Named(name.to_owned());
        }
        if let Some(scm) = s.strip_prefix("scm:") {
            let (vcs, mergebase) = scm.split_once(':').unwrap_or((scm, ""));
            let vcs = match vcs {
                "git" => ScmVcs::Git,
                "hg" | "sapling" | "sl" => ScmVcs::Hg,
                _ => return ClockSpec::Epoch,
            };
            return ClockSpec::Scm {
                vcs,
                mergebase: mergebase.to_owned(),
            };
        }
        if let Ok(n) = s.parse::<u64>() {
            return ClockSpec::Tick(n);
        }
        ClockSpec::Epoch
    }

    /// Tick comparison for `since` queries.  A spec pointing at a
    /// different root instance (mismatched start/pid/root) resets to 0
    /// so the query returns every file — same as watchman.
    pub fn tick_against(&self, clock: &Clock) -> u64 {
        match self {
            ClockSpec::Epoch => 0,
            // Named and SCM cursors are orthogonal to the tick axis;
            // they filter by path set, not by tick.  The query engine
            // treats tick=0 as "consider every file" and then narrows
            // down via the cursor's file set.
            ClockSpec::Named(_) | ClockSpec::Scm { .. } => 0,
            ClockSpec::Tick(t) => *t,
            ClockSpec::Full {
                start_time,
                pid,
                root_number,
                tick,
            } => {
                if *start_time == clock.start_time
                    && *pid == clock.pid
                    && *root_number == clock.root_number
                {
                    *tick
                } else {
                    0
                }
            }
        }
    }
}
