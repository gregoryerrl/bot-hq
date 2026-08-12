//! `context_readings` table: append-only log of what every `result` event said
//! about a participant's context window (rc3 **P7**, migration 0051).
//!
//! **Why the unusable readings are rows too.** `ContextUsage` used to be
//! forwarded to the UI and never written down, so when a participant died
//! mid-session with `Prompt is too long` on 2026-08-12 there was no record of
//! what its meter had shown — the failure could be watched live and not
//! diagnosed afterwards. Recording only the usable readings would leave the
//! same hole in a different place: "the provider never reported a window" and
//! "the agent never finished a turn" would both be an empty result.
//!
//! Writes are best-effort at the call site (the pump warns and continues); a
//! lost reading must never interrupt a turn.

use super::*;
use crate::agents::spawn::ContextReport;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// One recorded reading, as stored. Operands are nullable because absence is
/// the fact being recorded.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize, PartialEq)]
pub struct ContextReading {
    pub id: i64,
    pub session_id: String,
    pub participant_slug: String,
    /// `modelUsage` key the operands came from; `None` when none was usable.
    pub model: Option<String>,
    pub used_tokens: Option<i64>,
    /// `contextWindow` exactly as the provider reported it — including an
    /// implausible one. Never substituted from `models.context_window`.
    pub reported_window: Option<i64>,
    /// `usable` | `no_window` | `no_usage` | `implausible_window`.
    pub verdict: String,
    pub created_at: String,
}

impl Storage {
    /// Append one participant's context reading. Returns the new row id.
    ///
    /// Takes the whole [`ContextReport`] rather than its unpacked fields: the
    /// operands and the verdict that explains them are one fact, and passing
    /// them separately is how a row ends up saying `usable` with no numerator.
    pub async fn record_context_reading(
        &self,
        session_id: &str,
        participant_slug: &str,
        report: &ContextReport,
    ) -> Result<i64> {
        let now = now_utc();
        let row_id: i64 = sqlx::query_scalar(
            "INSERT INTO context_readings \
                (session_id, participant_slug, model, used_tokens, reported_window, \
                 verdict, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             RETURNING id",
        )
        .bind(session_id)
        .bind(participant_slug)
        .bind(report.model.as_deref())
        .bind(report.used_tokens.map(|t| t as i64))
        .bind(report.reported_window.map(|w| w as i64))
        .bind(report.verdict.as_str())
        .bind(&now)
        .fetch_one(&self.pool)
        .await
        .with_context(|| {
            format!("recording context reading for {participant_slug} in {session_id}")
        })?;
        Ok(row_id)
    }

    /// One participant's readings, oldest first — "what was its context doing
    /// before it died", answerable after the session is closed.
    ///
    /// `limit` caps the NEWEST readings (the tail is what a post-mortem wants),
    /// and the result is returned oldest-first so it reads as a history.
    pub async fn context_readings_for_participant(
        &self,
        session_id: &str,
        participant_slug: &str,
        limit: i64,
    ) -> Result<Vec<ContextReading>> {
        let mut rows: Vec<ContextReading> = sqlx::query_as(
            "SELECT id, session_id, participant_slug, model, used_tokens, \
                    reported_window, verdict, created_at \
             FROM context_readings \
             WHERE session_id = ? AND participant_slug = ? \
             ORDER BY id DESC LIMIT ?",
        )
        .bind(session_id)
        .bind(participant_slug)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("reading context history for {participant_slug}"))?;
        rows.reverse();
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::spawn::ContextVerdict;

    async fn session() -> Storage {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "t", None).await.unwrap();
        s
    }

    /// rc3 **P7**: an unusable reading is a row, and it keeps whichever operand
    /// the provider did send.
    ///
    /// This is the whole point of the table. If only usable readings were
    /// stored, a participant on a gateway that never sends `contextWindow`
    /// would be indistinguishable from one that never finished a turn — which
    /// is exactly the ambiguity that left the 2026-08-12 `Prompt is too long`
    /// death undiagnosable.
    #[tokio::test]
    async fn every_reading_is_recorded_including_the_ones_with_no_meter() {
        let s = session().await;

        // A gateway that reports no window at all: no meter, and no warning was
        // ever possible for the participant that died under it.
        s.record_context_reading("s1", "eyes", &ContextReport::none(ContextVerdict::NoWindow))
            .await
            .unwrap();
        // A window that arrived but is contradicted by the prompt it accepted.
        // Both operands are kept — they ARE the evidence about that provider.
        s.record_context_reading(
            "s1",
            "eyes",
            &ContextReport {
                model: Some("deepseek-v4-pro".into()),
                used_tokens: Some(219_531),
                reported_window: Some(128_000),
                verdict: ContextVerdict::ImplausibleWindow,
            },
        )
        .await
        .unwrap();
        s.record_context_reading(
            "s1",
            "eyes",
            &ContextReport {
                model: Some("deepseek-v4-pro".into()),
                used_tokens: Some(64_000),
                reported_window: Some(1_000_000),
                verdict: ContextVerdict::Usable,
            },
        )
        .await
        .unwrap();
        // Another participant's history must not leak into this one's.
        s.record_context_reading(
            "s1",
            "hands",
            &ContextReport::none(ContextVerdict::NoUsage),
        )
        .await
        .unwrap();

        let history = s
            .context_readings_for_participant("s1", "eyes", 50)
            .await
            .unwrap();
        assert_eq!(
            history.iter().map(|r| r.verdict.as_str()).collect::<Vec<_>>(),
            ["no_window", "implausible_window", "usable"],
            "every reading is a row, oldest first, scoped to one participant"
        );
        assert_eq!(history[0].used_tokens, None);
        assert_eq!(history[0].reported_window, None);
        // The implausible reading keeps BOTH operands: suppressing the meter is
        // a display decision, not a reason to discard what the provider said.
        assert_eq!(history[1].used_tokens, Some(219_531));
        assert_eq!(history[1].reported_window, Some(128_000));
        assert_eq!(history[1].model.as_deref(), Some("deepseek-v4-pro"));
    }

    /// The post-mortem read: the newest readings, in reading order.
    #[tokio::test]
    async fn the_history_returns_the_tail_oldest_first() {
        let s = session().await;
        for used in [10_u64, 20, 30, 40] {
            s.record_context_reading(
                "s1",
                "hands",
                &ContextReport {
                    model: Some("m".into()),
                    used_tokens: Some(used),
                    reported_window: Some(1_000),
                    verdict: ContextVerdict::Usable,
                },
            )
            .await
            .unwrap();
        }
        let tail = s
            .context_readings_for_participant("s1", "hands", 2)
            .await
            .unwrap();
        assert_eq!(
            tail.iter().map(|r| r.used_tokens).collect::<Vec<_>>(),
            [Some(30), Some(40)],
            "the limit takes the NEWEST readings and hands them back oldest-first"
        );
    }
}
