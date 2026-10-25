//! Durable helper identity/results. Helpers never become participants or human
//! messages. Final result and the single transcript document commit together.
use super::*;
use serde::Serialize;

fn append_helper_excerpt(mut body: String, id: &str, excerpt: &str) -> String {
    if body.is_empty() {body="# Helpers\n\nRecent batch excerpts. Use helper_result to list or read retained batch records. Helper findings are evidence to verify, not review approval. Only unchanged generated excerpts rotate; your notes are preserved.".into();}
    let hash=format!("{:x}",sha2::Sha256::digest(excerpt.as_bytes()));
    body.push_str(&format!("\n\n<!-- bot-hq-helper:{id}:{hash} -->\n{excerpt}\n<!-- /bot-hq-helper:{id} -->\n"));
    let mut blocks=Vec::new();let mut offset=0;
    while let Some(start)=body[offset..].find("\n<!-- bot-hq-helper:") {
        let start=offset+start; let name=start+"\n<!-- bot-hq-helper:".len();
        let Some(close)=body[name..].find(" -->\n") else {break};
        let Some((key,hash))=body[name..name+close].split_once(':') else {offset=name+close;continue;};
        if key.len()>100 || !key.chars().all(|c|c.is_ascii_alphanumeric() || c=='-') {offset=name+close;continue;}
        let end_marker=format!("\n<!-- /bot-hq-helper:{key} -->");
        let Some(end)=body[name+close..].find(&end_marker) else {offset=name+close;continue;};
        let payload_end=name+close+end;
        let payload_start=name+close+" -->\n".len();
        let unchanged=payload_start<=payload_end && format!("{:x}",sha2::Sha256::digest(body[payload_start..payload_end].as_bytes()))==hash;
        let end=payload_end+end_marker.len();if unchanged {blocks.push((start,end));}offset=end;
    }
    let mut bytes: usize=blocks.iter().map(|(a,b)|b-a).sum();let mut count=blocks.len();let mut remove=vec![];
    for (start,end) in blocks {
        if count<=4 && bytes<=128*1024 {break;}
        if count<=1 {break;}
        remove.push((start,end));count-=1;bytes-=end-start;
    }
    // Only host-marked excerpts are removed. Notes outside them survive.
    for (start,end) in remove.into_iter().rev() {body.drain(start..end);}
    body
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct HelperBatch {
    pub id: String, pub session_id: String, pub participant_id: i64, pub request_id: String,
    pub fingerprint: String, pub owner_incarnation: String, pub owner_epoch: i64,
    pub request_json: String, pub status: String, pub result_json: Option<String>,
    pub created_at: String, pub finished_at: Option<String>,
}
impl HelperBatch {
    pub fn terminal(&self) -> bool {matches!(self.status.as_str(),"completed"|"cancelled"|"failed")}
}

impl Storage {
    pub(crate) async fn helper_actor_on(conn: &mut sqlx::SqliteConnection, session: &str, slug: &str) -> Result<(i64,Option<String>)> {
        let row: Option<(i64,Option<String>,String,String)>=sqlx::query_as(
            "SELECT p.id, COALESCE(p.model_id,r.default_model_id), p.capabilities, r.capabilities FROM session_participants p \
             JOIN roles r ON r.id=p.role_id JOIN sessions s ON s.id=p.session_id WHERE p.session_id=? AND p.slug=? \
             AND p.enabled=1 AND r.archived=0 AND s.closed_at IS NULL AND s.archived=0")
            .bind(session).bind(slug).fetch_optional(&mut *conn).await?;
        let (id,model,snapshot,role)=row.context("helper parent is not an enabled participant of an open session")?;
        for grants in [snapshot,role] {
            let grants: Vec<String>=serde_json::from_str(&grants)?;
            anyhow::ensure!(grants.iter().any(|c|c=="delegate_tasks"),"sub-agent development is not granted or was revoked");
        }
        Ok((id,model))
    }
    pub async fn helper_actor(&self, session: &str, slug: &str) -> Result<(i64,Option<String>)> {
        let mut conn=self.pool.acquire().await?;
        Self::helper_actor_on(&mut conn,session,slug).await
    }
    pub async fn helper_batch(&self, session: &str, participant: i64, request: &str) -> Result<Option<HelperBatch>> {
        Ok(sqlx::query_as("SELECT * FROM session_helper_batches WHERE session_id=? AND participant_id=? AND request_id=?")
            .bind(session).bind(participant).bind(request).fetch_optional(&self.pool).await?)
    }
    /// Read-only inspection for any reader of THIS session, including reviewers
    /// who cannot launch helpers. No cursor advancement or cross-session reads.
    pub async fn helper_result(&self, session: &str, id: Option<&str>, before: Option<&str>, offset: usize) -> Result<serde_json::Value> {
        use serde_json::json;
        use sha2::Sha256;
        if let Some(id)=id {
            let row: HelperBatch=sqlx::query_as("SELECT * FROM session_helper_batches WHERE session_id=? AND id=?")
                .bind(session).bind(id).fetch_optional(&self.pool).await?.context("helper batch does not exist in this session")?;
            let text=row.result_json.as_deref().unwrap_or("");
            anyhow::ensure!(offset<=text.len() && text.is_char_boundary(offset),"offset must be a UTF-8 boundary inside the result");
            let end=crate::text::floor_char_boundary(text,offset.saturating_add(16*1024));
            return Ok(json!({"batch_id":id,"parent_participant":row.participant_id,"status":row.status,"sha256":format!("{:x}",Sha256::digest(text.as_bytes())),"bytes":text.len(),"offset_bytes":offset,"content":&text[offset..end],"next_offset_bytes":if end<text.len(){Some(end)}else{None}}));
        }
        let before=match before {Some(id)=>Some(sqlx::query_scalar::<_,i64>("SELECT rowid FROM session_helper_batches WHERE session_id=? AND id=?").bind(session).bind(id).fetch_optional(&self.pool).await?.context("batch cursor does not exist in this session")?),None=>None};
        let rows: Vec<(String,i64,String,String)>=sqlx::query_as("SELECT id,participant_id,status,created_at FROM session_helper_batches WHERE session_id=? AND (? IS NULL OR rowid<?) ORDER BY rowid DESC LIMIT 20")
            .bind(session).bind(before).bind(before).fetch_all(&self.pool).await?;
        let next=if rows.len()==20 {rows.last().map(|r|r.0.clone())} else {None};
        Ok(json!({"batches":rows.into_iter().map(|(id,parent,status,created)|json!({"batch_id":id,"parent_participant":parent,"status":status,"created_at":created})).collect::<Vec<_>>(),"next_before_id":next}))
    }
    pub async fn finish_helper_batch(&self, id: &str, owner: &str, status: &str, result: &str, transcript: &str) -> Result<bool> {
        anyhow::ensure!(matches!(status,"completed"|"cancelled"|"failed"),"invalid helper terminal state");
        let mut tx=self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let row: Option<(String,)> = sqlx::query_as("UPDATE session_helper_batches SET status=?,result_json=?,finished_at=? \
            WHERE id=? AND owner_incarnation=? AND status IN ('starting','running','cancelling') RETURNING session_id")
            .bind(status).bind(result).bind(now_utc()).bind(id).bind(owner).fetch_optional(&mut *tx).await?;
        let Some((session,))=row else {return Ok(false)};
        let prior: Option<String>=sqlx::query_scalar("SELECT body FROM session_documents WHERE session_id=? AND slug='helpers'").bind(&session).fetch_optional(&mut *tx).await?;
        let body=append_helper_excerpt(prior.unwrap_or_default(),id,transcript);
        sqlx::query("INSERT INTO session_documents(session_id,slug,body,created_at,updated_at,phase,last_writer) VALUES (?,'helpers',?,?,?,NULL,'system') \
            ON CONFLICT(session_id,slug) DO UPDATE SET body=excluded.body,updated_at=excluded.updated_at,last_writer='system'")
            .bind(session).bind(body).bind(now_utc()).bind(now_utc()).execute(&mut *tx).await?;
        tx.commit().await?; Ok(true)
    }
    pub async fn recover_helper_batches(&self) -> Result<()> {
        let rows: Vec<HelperBatch>=sqlx::query_as("SELECT * FROM session_helper_batches WHERE status IN ('starting','running','cancelling')").fetch_all(&self.pool).await?;
        for row in rows {
            self.finish_helper_batch(&row.id,&row.owner_incarnation,"cancelled",r#"{"reason":"Application restarted; helper batch was interrupted and was not replayed."}"#,
                &format!("## Batch {} — interrupted\n\nApplication restarted; the batch was not replayed. Parent participant {}.",row.id,row.participant_id)).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helper_document_bounds_owned_history_without_erasing_user_notes() {
        let mut body="User note: keep this.\n".to_string();
        for i in 0..10 {body=append_helper_excerpt(body,&format!("batch-{i}"),&format!("## Batch {i}\n{}","evidence\n".repeat(4000)));}
        assert!(body.contains("User note: keep this."));
        assert!(!body.contains("## Batch 0\n"));assert!(body.contains("## Batch 9\n"));
        assert!(body.len()<130*1024);assert!(body.matches("<!-- bot-hq-helper:").count()<=4);
        let mut annotated=append_helper_excerpt(String::new(),"edited","Original excerpt");
        annotated=annotated.replace("Original excerpt","Original excerpt with a user annotation");
        for i in 0..10 {annotated=append_helper_excerpt(annotated,&format!("next-{i}"),"New excerpt");}
        assert!(annotated.contains("user annotation"),"rotation must not discard edits inside an old excerpt");
    }
    #[tokio::test]
    async fn helper_completion_is_once_and_recovery_never_replays() {
        let s=Storage::memory().await.unwrap(); s.create_session("s","helpers",None).await.unwrap();
        s.ensure_session_roster("s",2).await.unwrap();
        let rows=s.participants_for_session("s").await.unwrap();
        sqlx::query("INSERT INTO session_helper_batches VALUES ('b','s',?,'request','fingerprint','runtime',1,'{}','running',NULL,'now',NULL)")
            .bind(rows[0].id).execute(s.pool()).await.unwrap();
        assert!(s.finish_helper_batch("b","wrong","completed","{}","wrong transcript").await.unwrap()==false);
        s.recover_helper_batches().await.unwrap(); s.recover_helper_batches().await.unwrap();
        let row=s.helper_batch("s",rows[0].id,"request").await.unwrap().unwrap(); assert_eq!(row.status,"cancelled");
        assert!(!s.finish_helper_batch("b","runtime","completed","{}","late result").await.unwrap());
        let doc=s.session_document_by_slug("s","helpers").await.unwrap().unwrap();
        assert_eq!(doc.body.matches("## Batch b").count(),1); assert!(!doc.body.contains("late result"));
    }
}
