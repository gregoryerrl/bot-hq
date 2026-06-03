//! Timestamp helper. Every timestamp bot-hq persists is RFC3339 in UTC with a
//! `Z` designator (e.g. `2026-06-03T07:40:00.123Z`). That is the single
//! baseline: the frontend parses stored timestamps as UTC and only converts to
//! the viewer's local zone at render time, and agents are told to reason about
//! staleness in UTC.
//!
//! Why this exists: SQLite's `datetime('now')` / `CURRENT_TIMESTAMP` emit a
//! ZONE-LESS string (`2026-06-03 07:40:00`). JavaScript's `new Date(iso)`
//! parses a zone-less string as *local* time, so on a UTC+8 machine a freshly
//! created row reads "8h ago" — the staleness hallucination. We therefore write
//! timestamps from Rust as RFC3339-Z instead of leaning on SQLite defaults.

use chrono::{SecondsFormat, Utc};

/// Current instant as RFC3339 UTC with a `Z` suffix, millisecond precision.
pub(crate) fn now_utc() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_utc_is_zone_explicit_and_parseable() {
        let s = now_utc();
        assert!(s.ends_with('Z'), "expected Z suffix, got {s}");
        assert!(s.contains('T'), "expected RFC3339 T separator, got {s}");
        // Round-trips through a real RFC3339 parse as UTC.
        let parsed = chrono::DateTime::parse_from_rfc3339(&s).expect("valid rfc3339");
        assert_eq!(parsed.offset().local_minus_utc(), 0, "must be UTC");
    }
}
