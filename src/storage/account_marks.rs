//! `account_marks` — the advisory usage-limit mark per Claude account and
//! model (migration 0091). See the migration for why it is keyed by config
//! dir + organisation id + model and why it never refuses anything.

use super::*;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct AccountMark {
    /// The config dir, `''` for the default `~/.claude`.
    pub config_dir: String,
    /// The organisation id the CLI reported for the dir at spawn, `''` when
    /// unknown.
    pub org_id: String,
    /// The wire model id the participant that hit the limit ran on —
    /// claude.ai limits are per model.
    pub model_name: String,
    /// The signed-in email at spawn, for display.
    pub email: Option<String>,
    /// RFC 3339 UTC, or `None` when the line carried no reset and no fallback
    /// applied (the credits message).
    pub limited_until: Option<String>,
    /// The CLI's own line, verbatim.
    pub limited_text: String,
    /// RFC 3339 UTC.
    pub marked_at: String,
}

impl Storage {
    pub async fn list_account_marks(&self) -> Result<Vec<AccountMark>> {
        let rows = sqlx::query_as::<_, AccountMark>(
            "SELECT config_dir, org_id, model_name, email, limited_until, limited_text, marked_at \
             FROM account_marks ORDER BY marked_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .context("listing account marks")?;
        Ok(rows)
    }

    pub async fn get_account_mark(
        &self,
        config_dir: &str,
        org_id: &str,
        model_name: &str,
    ) -> Result<Option<AccountMark>> {
        let row = sqlx::query_as::<_, AccountMark>(
            "SELECT config_dir, org_id, model_name, email, limited_until, limited_text, marked_at \
             FROM account_marks WHERE config_dir = ? AND org_id = ? AND model_name = ?",
        )
        .bind(config_dir)
        .bind(org_id)
        .bind(model_name)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Mark the account + model limited now. A later limit on the same pair
    /// replaces the earlier mark — the newest reset is the one that counts.
    pub async fn set_account_mark(
        &self,
        config_dir: &str,
        org_id: &str,
        model_name: &str,
        email: Option<&str>,
        limited_until: Option<&str>,
        limited_text: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO account_marks (config_dir, org_id, model_name, email, limited_until, limited_text, marked_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(config_dir, org_id, model_name) DO UPDATE SET \
                email = excluded.email, \
                limited_until = excluded.limited_until, \
                limited_text = excluded.limited_text, \
                marked_at = excluded.marked_at",
        )
        .bind(config_dir)
        .bind(org_id)
        .bind(model_name)
        .bind(email)
        .bind(limited_until)
        .bind(limited_text)
        .bind(now_utc())
        .execute(&self.pool)
        .await
        .context("marking an account limited")?;
        Ok(())
    }

    /// Remove the mark. Returns whether one was there.
    pub async fn clear_account_mark(
        &self,
        config_dir: &str,
        org_id: &str,
        model_name: &str,
    ) -> Result<bool> {
        let res = sqlx::query(
            "DELETE FROM account_marks WHERE config_dir = ? AND org_id = ? AND model_name = ?",
        )
        .bind(config_dir)
        .bind(org_id)
        .bind(model_name)
            .execute(&self.pool)
            .await
            .context("clearing an account mark")?;
        Ok(res.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_mark_round_trips_replaces_and_clears() {
        let s = Storage::memory().await.unwrap();
        let fable = "claude-fable-5-1";
        assert_eq!(s.get_account_mark("", "org-1", fable).await.unwrap(), None);
        s.set_account_mark("", "org-1", fable, Some("a@x"), Some("2026-10-05T06:00:00Z"), "weekly limit")
            .await
            .unwrap();
        let m = s.get_account_mark("", "org-1", fable).await.unwrap().unwrap();
        assert_eq!(m.limited_until.as_deref(), Some("2026-10-05T06:00:00Z"));
        assert_eq!(m.limited_text, "weekly limit");
        assert_eq!(m.email.as_deref(), Some("a@x"));
        assert_eq!(m.model_name, fable);
        // The same dir under another org is another account; the same account
        // on another model is another limit (EYES 6c45b98e).
        assert_eq!(s.get_account_mark("", "org-2", fable).await.unwrap(), None);
        assert_eq!(s.get_account_mark("", "org-1", "claude-opus-5-5").await.unwrap(), None);
        s.set_account_mark("", "org-1", fable, None, None, "out of credits").await.unwrap();
        let m = s.get_account_mark("", "org-1", fable).await.unwrap().unwrap();
        assert_eq!(m.limited_until, None);
        assert_eq!(m.limited_text, "out of credits");
        assert_eq!(s.list_account_marks().await.unwrap().len(), 1);
        assert!(s.clear_account_mark("", "org-1", fable).await.unwrap());
        assert!(!s.clear_account_mark("", "org-1", fable).await.unwrap());
        assert_eq!(s.get_account_mark("", "org-1", fable).await.unwrap(), None);
    }
}
