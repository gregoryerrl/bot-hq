//! Host-side iframe heartbeat watcher.
//!
//! Architecture (per design doc § Error Handling 3):
//!
//! 1. Heartbeat watchers register at the **app-shell level** (i.e., the
//!    Dashboard route — never unmounts). NOT inside per-session
//!    `<PluginSlot>` components — those remount, which would kill the
//!    watcher and let a crashed plugin run unwatched.
//! 2. Every 5s the host sends a `__ping` message; the iframe is expected
//!    to reply with `__pong` within 1s.
//! 3. Three consecutive misses → crash recovery: host removes the iframe +
//!    surfaces a fallback. (v1 = manual reload from PluginManager;
//!    pre-third-party = exponential-backoff auto-restart.)
//!
//! The actual ping/pong message-passing happens via the `Window.postMessage`
//! frontend channel; this Rust module owns the timing + state machine + a
//! callback the frontend wiring invokes on each miss/recover.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const PING_INTERVAL: Duration = Duration::from_secs(5);
const PONG_WINDOW: Duration = Duration::from_secs(1);
const MISS_LIMIT: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginStatus {
    /// Last ping got a pong within the window.
    Healthy,
    /// One or more misses, below MISS_LIMIT.
    Slow { miss_count: u32 },
    /// MISS_LIMIT consecutive misses — host should tear down the iframe.
    Crashed,
}

#[derive(Debug)]
struct PluginState {
    last_ping_at: Option<Instant>,
    miss_count: u32,
    status: PluginStatus,
}

impl PluginState {
    fn new() -> Self {
        Self {
            last_ping_at: None,
            miss_count: 0,
            status: PluginStatus::Healthy,
        }
    }
}

pub struct Heartbeat {
    plugins: Mutex<HashMap<String, PluginState>>,
}

impl Heartbeat {
    pub fn new() -> Self {
        Self {
            plugins: Mutex::new(HashMap::new()),
        }
    }

    /// Register a plugin for monitoring. Idempotent on existing IDs.
    pub fn register(&self, plugin_id: &str) {
        let mut g = self.plugins.lock().unwrap_or_else(|p| p.into_inner());
        g.entry(plugin_id.to_string()).or_insert_with(PluginState::new);
    }

    /// Drop a plugin from monitoring (e.g., user uninstalled).
    pub fn unregister(&self, plugin_id: &str) {
        let mut g = self.plugins.lock().unwrap_or_else(|p| p.into_inner());
        g.remove(plugin_id);
    }

    /// Record that we just sent a ping. Resets any pending miss-detection.
    pub fn note_ping_sent(&self, plugin_id: &str) {
        let mut g = self.plugins.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(state) = g.get_mut(plugin_id) {
            state.last_ping_at = Some(Instant::now());
        }
    }

    /// Iframe responded with a pong. Resets miss counter.
    pub fn note_pong_received(&self, plugin_id: &str) {
        let mut g = self.plugins.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(state) = g.get_mut(plugin_id) {
            state.miss_count = 0;
            state.status = PluginStatus::Healthy;
            state.last_ping_at = None;
        }
    }

    /// Sweep all registered plugins. For each plugin whose last_ping_at is
    /// older than PONG_WINDOW, bump miss_count. Returns the list of plugin
    /// IDs that crossed into [`PluginStatus::Crashed`] this tick — the
    /// caller should tear down those iframes.
    pub fn sweep(&self) -> Vec<String> {
        let mut g = self.plugins.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        let mut newly_crashed = Vec::new();

        for (id, state) in g.iter_mut() {
            if let Some(sent_at) = state.last_ping_at {
                if now.duration_since(sent_at) > PONG_WINDOW
                    && state.status != PluginStatus::Crashed
                {
                    state.miss_count += 1;
                    state.last_ping_at = None;
                    if state.miss_count >= MISS_LIMIT {
                        state.status = PluginStatus::Crashed;
                        newly_crashed.push(id.clone());
                    } else {
                        state.status = PluginStatus::Slow {
                            miss_count: state.miss_count,
                        };
                    }
                }
            }
        }

        newly_crashed
    }

    pub fn status_of(&self, plugin_id: &str) -> Option<PluginStatus> {
        let g = self.plugins.lock().unwrap_or_else(|p| p.into_inner());
        g.get(plugin_id).map(|s| s.status)
    }

    pub fn ping_interval() -> Duration {
        PING_INTERVAL
    }
}

impl Default for Heartbeat {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_plugin_is_healthy() {
        let hb = Heartbeat::new();
        hb.register("p1");
        assert_eq!(hb.status_of("p1"), Some(PluginStatus::Healthy));
    }

    #[test]
    fn pong_response_clears_miss_state() {
        let hb = Heartbeat::new();
        hb.register("p1");
        hb.note_ping_sent("p1");
        std::thread::sleep(Duration::from_millis(1100)); // > PONG_WINDOW
        hb.sweep();
        assert!(matches!(
            hb.status_of("p1"),
            Some(PluginStatus::Slow { miss_count: 1 })
        ));
        hb.note_pong_received("p1");
        assert_eq!(hb.status_of("p1"), Some(PluginStatus::Healthy));
    }

    #[test]
    fn three_consecutive_misses_mark_crashed() {
        let hb = Heartbeat::new();
        hb.register("p1");

        for _ in 0..3 {
            hb.note_ping_sent("p1");
            std::thread::sleep(Duration::from_millis(1100));
            hb.sweep();
        }

        assert_eq!(hb.status_of("p1"), Some(PluginStatus::Crashed));
    }

    #[test]
    fn sweep_returns_newly_crashed_ids() {
        let hb = Heartbeat::new();
        hb.register("p1");
        for _ in 0..2 {
            hb.note_ping_sent("p1");
            std::thread::sleep(Duration::from_millis(1100));
            hb.sweep();
        }
        // Now one more miss → crash.
        hb.note_ping_sent("p1");
        std::thread::sleep(Duration::from_millis(1100));
        let crashed = hb.sweep();
        assert_eq!(crashed, vec!["p1".to_string()]);

        // Subsequent sweep returns empty (don't re-emit).
        let crashed2 = hb.sweep();
        assert!(crashed2.is_empty());
    }

    #[test]
    fn unregister_drops_plugin_state() {
        let hb = Heartbeat::new();
        hb.register("p1");
        hb.unregister("p1");
        assert_eq!(hb.status_of("p1"), None);
    }
}
