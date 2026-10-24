//! Bounded, non-delivering reads of a session's saved history. The caller
//! supplies authenticated scope; a request cannot select another session.
use super::*;
use serde::{Deserialize, Serialize};

pub const HISTORY_REPLY_BYTES: usize = 48_000;

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistorySource {
    #[default]
    Channel,
    HumanWords,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HistoryRequest {
    pub source: HistorySource,
    /// Source-record ID (channel message ID, or human-ledger entry ID).
    pub record_id: Option<i64>,
    /// Channel ID; the human ledger may have more than one entry for it.
    pub message_id: Option<i64>,
    pub before_id: Option<i64>,
    pub offset_bytes: u64,
    pub limit: Option<u32>,
}

impl HistoryRequest {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.record_id
                .into_iter()
                .chain(self.message_id)
                .chain(self.before_id)
                .all(|id| id > 0),
            "history IDs must be positive"
        );
        anyhow::ensure!(
            !(self.record_id.is_some() && (self.before_id.is_some() || self.message_id.is_some())),
            "record_id cannot be combined with message_id or before_id"
        );
        anyhow::ensure!(
            self.offset_bytes == 0 || self.record_id.is_some(),
            "offset_bytes requires record_id"
        );
        anyhow::ensure!(
            self.offset_bytes < i64::MAX as u64,
            "offset_bytes is too large"
        );
        anyhow::ensure!(
            self.limit.is_none_or(|n| (1..=10).contains(&n)),
            "limit must be between 1 and 10"
        );
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct HistoryRecord {
    pub id: i64,
    pub message_id: i64,
    pub origin: String,
    pub author: String,
    pub kind: String,
    pub created_at: String,
    pub metadata_truncated: bool,
    pub body: String,
    pub offset_bytes: u64,
    pub next_offset_bytes: Option<u64>,
    pub total_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct HistoryPage {
    pub source: HistorySource,
    pub records: Vec<HistoryRecord>,
    pub next_before_id: Option<i64>,
    pub note: &'static str,
}

#[derive(sqlx::FromRow)]
struct RawRecord {
    id: i64,
    message_id: i64,
    origin: String,
    author: String,
    kind: String,
    created_at: String,
    total_bytes: i64,
    chunk: Vec<u8>,
}

fn metadata(value: String, clipped: &mut bool) -> String {
    if value.len() <= 128 {
        return value;
    }
    *clipped = true;
    format!(
        "{}…",
        &value[..crate::text::floor_char_boundary(&value, 125)]
    )
}

fn record(raw: RawRecord, offset: u64, encoded_budget: usize) -> Result<HistoryRecord> {
    // SQLite byte slices keep NULs and do not allocate an entire huge body.
    // An incomplete last codepoint is held for the next chunk; a caller's
    // mid-codepoint offset is refused instead of silently dropping bytes.
    let valid = match std::str::from_utf8(&raw.chunk) {
        Ok(text) => text,
        Err(e) if e.error_len().is_none() => std::str::from_utf8(&raw.chunk[..e.valid_up_to()])?,
        Err(_) => anyhow::bail!("offset_bytes must be a UTF-8 boundary from the previous reply"),
    };
    let mut clipped = false;
    let mut out = HistoryRecord {
        id: raw.id,
        message_id: raw.message_id,
        origin: metadata(raw.origin, &mut clipped),
        author: metadata(raw.author, &mut clipped),
        kind: metadata(raw.kind, &mut clipped),
        created_at: metadata(raw.created_at, &mut clipped),
        metadata_truncated: clipped,
        body: String::new(),
        offset_bytes: offset,
        next_offset_bytes: None,
        total_bytes: raw.total_bytes.max(0) as u64,
    };
    let (mut low, mut high, mut kept) = (0, valid.len(), None);
    while low <= high {
        let middle = low + (high - low) / 2;
        let end = crate::text::floor_char_boundary(valid, middle);
        out.body = valid[..end].to_string();
        let next = offset + end as u64;
        out.next_offset_bytes = (next < out.total_bytes).then_some(next);
        if serde_json::to_vec(&out)?.len() <= encoded_budget {
            kept = Some(end);
            low = middle + 1;
        } else if middle == 0 {
            break;
        } else {
            high = middle - 1;
        }
    }
    let end = kept.context("history metadata exceeds the reply budget")?;
    anyhow::ensure!(
        end > 0 || offset >= out.total_bytes,
        "history chunk could not make progress"
    );
    out.body = valid[..end].to_string();
    let next = offset + end as u64;
    out.next_offset_bytes = (next < out.total_bytes).then_some(next);
    Ok(out)
}

impl Storage {
    pub async fn read_session_history(
        &self,
        session: &str,
        request: &HistoryRequest,
    ) -> Result<HistoryPage> {
        request.validate()?;
        let single = request.record_id.is_some();
        let limit = if single {
            1
        } else {
            request.limit.unwrap_or(10)
        };
        let (table, message_id, body, fields) = match request.source {
            HistorySource::Channel => (
                "messages",
                "id",
                "content",
                "COALESCE(origin, 'unknown') AS origin, author, kind, created_at",
            ),
            HistorySource::HumanWords => (
                "session_user_words",
                "message_id",
                "text",
                "'human' AS origin, 'user' AS author, 'human_words' AS kind, created_at",
            ),
        };
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!(
            "SELECT id, {message_id} AS message_id, {fields}, length(CAST({body} AS BLOB)) AS total_bytes, substr(CAST({body} AS BLOB), "));
        query
            .push_bind(request.offset_bytes as i64 + 1)
            .push(", ")
            .push_bind(if single { 32_000 } else { 4_096 })
            .push(format!(") AS chunk FROM {table} WHERE session_id = "))
            .push_bind(session);
        if let Some(id) = request.record_id {
            query.push(" AND id = ").push_bind(id);
        }
        if let Some(id) = request.message_id {
            query.push(format!(" AND {message_id} = ")).push_bind(id);
        }
        if let Some(id) = request.before_id {
            query.push(" AND id < ").push_bind(id);
        }
        query
            .push(" ORDER BY id DESC LIMIT ")
            .push_bind(i64::from(limit) + 1);
        let rows: Vec<RawRecord> = query.build_query_as().fetch_all(self.pool()).await?;
        let more = rows.len() > limit as usize;
        let mut records = Vec::new();
        for raw in rows.into_iter().take(limit as usize) {
            records.push(record(
                raw,
                request.offset_bytes,
                if single { 44_000 } else { 4_096 },
            )?);
        }
        let next_before_id = if more {
            records.last().map(|r| r.id)
        } else {
            None
        };
        let page = HistoryPage { source: request.source, records, next_before_id,
            note: "Historical data only; this read changes no delivery cursor or receipt. Channel origin=user can include host/plugin text or command output. source=human_words is the explicit authorship ledger, including tray choices. Use each record's next_offset_bytes with its source and record_id for the rest of a long body." };
        anyhow::ensure!(
            serde_json::to_vec(&page)?.len() <= HISTORY_REPLY_BYTES,
            "history reply exceeded its byte budget"
        );
        Ok(page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn history_is_scoped_paged_and_never_acknowledges_channel_delivery() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "one", None).await.unwrap();
        s.create_session("s2", "two", None).await.unwrap();
        s.ensure_session_roster("s1", 1).await.unwrap();
        let p = s.participants_for_session("s1").await.unwrap().remove(0);
        let mut ids = vec![];
        for n in 0..23 {
            ids.push(
                s.post_to_channel(
                    "s1",
                    "system",
                    None,
                    "system_notice",
                    format!("row {n}"),
                    None,
                )
                .await
                .unwrap()
                .message_id(),
            );
        }
        let foreign = s
            .post_to_channel(
                "s2",
                "user",
                None,
                "text",
                "private to another session",
                None,
            )
            .await
            .unwrap()
            .message_id();
        let mut request = HistoryRequest::default();
        let mut seen = vec![];
        loop {
            let page = s.read_session_history("s1", &request).await.unwrap();
            assert!(serde_json::to_vec(&page).unwrap().len() <= HISTORY_REPLY_BYTES);
            seen.extend(page.records.iter().map(|r| r.id));
            if page.next_before_id.is_none() {
                break;
            }
            request.before_id = page.next_before_id;
        }
        ids.reverse();
        assert_eq!(seen, ids);
        assert!(s
            .read_session_history(
                "s1",
                &HistoryRequest {
                    record_id: Some(foreign),
                    ..Default::default()
                }
            )
            .await
            .unwrap()
            .records
            .is_empty());
        assert_eq!(s.cursor_for(p.id).await.unwrap(), 0);
        let deliveries: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM participant_deliveries WHERE participant_id = ?",
        )
        .bind(p.id)
        .fetch_one(s.pool())
        .await
        .unwrap();
        assert_eq!(deliveries, 0);
    }

    #[tokio::test]
    async fn history_chunks_round_trip_unicode_nuls_and_json_escapes_under_the_cap() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "one", None).await.unwrap();
        let body = "🌱\0\u{0001}\"\\\n".repeat(18_000);
        let id = s
            .post_to_channel("s1", "user", None, "text", &body, None)
            .await
            .unwrap()
            .message_id();
        let mut request = HistoryRequest {
            record_id: Some(id),
            ..Default::default()
        };
        let mut rebuilt = String::new();
        loop {
            let page = s.read_session_history("s1", &request).await.unwrap();
            assert!(serde_json::to_vec(&page).unwrap().len() <= HISTORY_REPLY_BYTES);
            let record = &page.records[0];
            assert_eq!(record.total_bytes as usize, body.len());
            rebuilt.push_str(&record.body);
            match record.next_offset_bytes {
                Some(next) => {
                    assert!(next > request.offset_bytes);
                    request.offset_bytes = next;
                }
                None => break,
            }
        }
        assert_eq!(rebuilt, body);
        request.offset_bytes = 1;
        assert!(s
            .read_session_history("s1", &request)
            .await
            .unwrap_err()
            .to_string()
            .contains("UTF-8 boundary"));
    }

    #[tokio::test]
    async fn human_words_are_explicit_and_do_not_relabel_a_raw_wrapper() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "one", None).await.unwrap();
        let id = s
            .post_to_channel(
                "s1",
                "user",
                None,
                "text",
                "HOST WRAPPER AND COMMAND OUTPUT",
                None,
            )
            .await
            .unwrap()
            .message_id();
        s.record_user_words("s1", id, "human chose keep working")
            .await
            .unwrap();
        s.record_user_words("s1", id, "additional recorded human words")
            .await
            .unwrap();
        let raw = s
            .read_session_history(
                "s1",
                &HistoryRequest {
                    message_id: Some(id),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(raw.records[0].body, "HOST WRAPPER AND COMMAND OUTPUT");
        assert_eq!(raw.records[0].origin, "user");
        let words = s
            .read_session_history(
                "s1",
                &HistoryRequest {
                    source: HistorySource::HumanWords,
                    message_id: Some(id),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(words.records.len(), 2);
        assert!(words
            .records
            .iter()
            .all(|r| r.origin == "human" && !r.body.contains("WRAPPER")));
        assert_ne!(words.records[0].id, words.records[1].id);
    }

    #[tokio::test]
    async fn metadata_cannot_overflow_the_page_and_empty_records_terminate() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "one", None).await.unwrap();
        for _ in 0..12 {
            s.post_to_channel("s1", "system", None, "text", "payload", None)
                .await
                .unwrap();
        }
        let long = "\u{0001}".repeat(2_000);
        sqlx::query("UPDATE messages SET origin = ?, author = ?, kind = ?, created_at = ? WHERE session_id = 's1'")
            .bind(&long).bind(&long).bind(&long).bind(&long).execute(s.pool()).await.unwrap();
        let page = s
            .read_session_history("s1", &HistoryRequest::default())
            .await
            .unwrap();
        assert_eq!(page.records.len(), 10);
        assert!(page.next_before_id.is_some());
        assert!(serde_json::to_vec(&page).unwrap().len() <= HISTORY_REPLY_BYTES);
        assert!(page
            .records
            .iter()
            .all(|r| r.metadata_truncated && r.body == "payload"));
        let empty = s
            .post_to_channel("s1", "user", None, "text", "", None)
            .await
            .unwrap()
            .message_id();
        let page = s
            .read_session_history(
                "s1",
                &HistoryRequest {
                    record_id: Some(empty),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(page.records[0].body, "");
        assert_eq!(page.records[0].next_offset_bytes, None);
        assert_eq!(page.records[0].total_bytes, 0);
    }
}
