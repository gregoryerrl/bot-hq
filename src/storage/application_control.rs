//! Explicit controller identity and its durable application-action audit.
use super::*;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ControlActor {
    pub session_id: String,
    pub participant_id: i64,
    pub slug: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ControlAudit {
    pub id: i64,
    pub source_session: String,
    pub source_participant: i64,
    pub source_slug: String,
    pub target_session: Option<String>,
    pub operation: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub outcome: String,
    pub detail: Option<String>,
}

impl Storage {
    /// Invite-time grant AND current role consent. Granting a role later does
    /// not upgrade existing participants; revocation takes effect immediately.
    pub async fn controller_actor(&self, session: &str, slug: &str) -> Result<ControlActor> {
        let mut conn = self.pool.acquire().await?;
        Self::controller_actor_on(&mut conn, session, slug).await
    }

    pub(crate) async fn controller_actor_on(
        conn: &mut sqlx::SqliteConnection,
        session: &str,
        slug: &str,
    ) -> Result<ControlActor> {
        let row: Option<(i64, String, String)> = sqlx::query_as(
            "SELECT p.id, p.capabilities, r.capabilities FROM session_participants p \
             JOIN sessions s ON s.id = p.session_id JOIN roles r ON r.id = p.role_id \
             WHERE p.session_id = ? AND p.slug = ? AND p.enabled <> 0 \
             AND r.archived = 0 AND s.closed_at IS NULL AND s.archived = 0",
        )
        .bind(session)
        .bind(slug)
        .fetch_optional(&mut *conn)
        .await?;
        let (participant_id, snapshot, live) =
            row.context("controller is no longer an enabled participant of an open session")?;
        for grants in [snapshot, live] {
            let values: Vec<String> =
                serde_json::from_str(&grants).context("controller grants cannot be read")?;
            anyhow::ensure!(
                values.iter().any(|s| s == "control_bot_hq"),
                "full application control is not granted or has been revoked"
            );
        }
        Ok(ControlActor {
            session_id: session.into(),
            participant_id,
            slug: slug.into(),
        })
    }

    pub async fn start_control_audit(
        &self,
        actor: &ControlActor,
        target: Option<&str>,
        operation: &str,
    ) -> Result<i64> {
        Ok(sqlx::query("INSERT INTO application_control_audit \
            (source_session, source_participant, source_slug, target_session, operation, started_at, outcome) \
            VALUES (?, ?, ?, ?, ?, ?, 'started')")
            .bind(&actor.session_id).bind(actor.participant_id).bind(&actor.slug).bind(target)
            .bind(operation).bind(now_utc()).execute(&self.pool).await?.last_insert_rowid())
    }

    pub async fn finish_control_audit(&self, id: i64, error: Option<&str>) -> Result<()> {
        let detail = error.map(|s| crate::policy::secret_scan::redact_string(s.to_string()));
        let detail = detail.map(|s| s[..crate::text::floor_char_boundary(&s, 2000)].to_string());
        sqlx::query("UPDATE application_control_audit SET outcome = ?, detail = ?, finished_at = ? WHERE id = ? AND outcome = 'started'")
            .bind(if error.is_some() { "failed" } else { "ok" }).bind(detail).bind(now_utc()).bind(id)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn control_audit_page(
        &self,
        session: Option<&str>,
        before: Option<i64>,
    ) -> Result<Vec<ControlAudit>> {
        Ok(sqlx::query_as("SELECT * FROM application_control_audit \
            WHERE (? IS NULL OR source_session = ? OR target_session = ?) AND (? IS NULL OR id < ?) \
            ORDER BY id DESC LIMIT 20")
            .bind(session).bind(session).bind(session).bind(before).bind(before).fetch_all(&self.pool).await?)
    }
}
