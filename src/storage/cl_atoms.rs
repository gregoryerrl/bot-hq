//! `cl_atoms`: the FTS5 atom index backing queryable CL retrieval (Phase 3).
//!
//! An *atom* is one heading-delimited section of a CL file. The table is a
//! standalone FTS5 virtual table (`migrations/0024_cl_atoms.sql`): `heading_path`
//! and `body` are tokenized / BM25-searchable, the rest is UNINDEXED metadata.
//! Like `cl_index` it is a derived, disposable layer — `cl_rescan` repopulates it
//! from disk (`split_into_atoms` does the splitting). The ranked read side
//! (`cl_retrieve`) lands in Stage 2; this module owns the write side.

use super::*;

/// A heading-delimited section of a CL file, pre-insertion. `heading_path` is the
/// "H1 > H2" breadcrumb ("(intro)" for preamble before the first heading); `body`
/// is the section text. Produced by `signaling::bridge::util::split_into_atoms`,
/// consumed by [`Storage::replace_atoms_for_file`] (which computes the stored
/// SHA-256 `body_hash`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Atom {
    pub heading_path: String,
    pub body: String,
}

/// One atom returned by [`Storage::cl_retrieve`]: which file/section it came from
/// plus the body to inline. Distinct from [`Atom`] (the pre-insert form) by
/// carrying `file_path` so callers can cite the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievedAtom {
    pub file_path: String,
    pub heading_path: String,
    pub body: String,
}

impl Storage {
    /// Replace ALL atoms for one CL file in a single transaction: delete the
    /// file's existing atoms, then insert the freshly-split set. Called by
    /// `cl_rescan` when a file is added or its body changed on disk. Atomic so a
    /// mid-replace failure can't leave a half-updated file (stale + fresh atoms).
    pub async fn replace_atoms_for_file(
        &self,
        project_id: &str,
        file_path: &str,
        atoms: &[Atom],
        mtime: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM cl_atoms WHERE project_id = ? AND file_path = ?")
            .bind(project_id)
            .bind(file_path)
            .execute(&mut *tx)
            .await?;
        for atom in atoms {
            sqlx::query(
                "INSERT INTO cl_atoms(project_id, file_path, heading_path, body, mtime, body_hash) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(project_id)
            .bind(file_path)
            .bind(&atom.heading_path)
            .bind(&atom.body)
            .bind(mtime)
            .bind(atom_body_hash(&atom.body))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Delete every atom for a file — the `cl_rescan` orphan branch (file gone on
    /// disk). Returns the row count removed. The derived atom layer follows the
    /// index: when `cl_index` purges a dangling row, its atoms go too.
    pub async fn delete_atoms_for_file(&self, project_id: &str, file_path: &str) -> Result<u64> {
        let res = sqlx::query("DELETE FROM cl_atoms WHERE project_id = ? AND file_path = ?")
            .bind(project_id)
            .bind(file_path)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }

    /// Ranked body retrieval over the FTS atom index — the read side of Phase 3.
    /// Returns the atoms whose `heading_path`/`body` best match `query` (FTS5
    /// BM25), scoped to `project_id`, optionally restricted to `paths`, and
    /// accumulated under `budget_tokens` (a coarse ~chars/4 estimate). On a BM25
    /// tie, conventions/decisions atoms win (pin), then fresher atoms (mtime).
    /// The query is sanitized into a safe FTS5 MATCH expression so arbitrary input
    /// can't throw a syntax error; a query with no searchable tokens returns empty.
    pub async fn cl_retrieve(
        &self,
        project_id: &str,
        query: &str,
        paths: Option<&[String]>,
        budget_tokens: i64,
    ) -> Result<Vec<RetrievedAtom>> {
        let match_expr = to_fts5_match(query);
        if match_expr.is_empty() {
            return Ok(Vec::new());
        }
        let path_filter = paths.filter(|p| !p.is_empty());

        // Pin (conventions/decisions) + freshness are tie-breakers AFTER bm25
        // relevance. The pinned names are constant literals (no injection); the
        // optional path filter binds each path as a parameter. A safety LIMIT caps
        // rows pulled before the Rust-side token-budget trim.
        let mut sql = String::from(
            "SELECT file_path, heading_path, body FROM cl_atoms \
             WHERE project_id = ? AND cl_atoms MATCH ?",
        );
        if let Some(paths) = path_filter {
            sql.push_str(" AND file_path IN (");
            for i in 0..paths.len() {
                if i > 0 {
                    sql.push_str(", ");
                }
                sql.push('?');
            }
            sql.push(')');
        }
        sql.push_str(
            " ORDER BY bm25(cl_atoms), \
             CASE WHEN file_path IN ('conventions.md', 'decisions.md') THEN 0 ELSE 1 END, \
             mtime DESC LIMIT 128",
        );

        let mut q = sqlx::query_as::<_, (String, String, String)>(&sql)
            .bind(project_id)
            .bind(&match_expr);
        if let Some(paths) = path_filter {
            for p in paths {
                q = q.bind(p);
            }
        }
        let rows = q.fetch_all(&self.pool).await?;

        // Accumulate under the budget; always keep at least the top atom so one
        // oversized atom can't make the whole retrieval return empty.
        let mut out = Vec::new();
        let mut used = 0i64;
        for (file_path, heading_path, body) in rows {
            let cost = estimate_tokens(&body);
            if !out.is_empty() && used + cost > budget_tokens {
                break;
            }
            used += cost;
            out.push(RetrievedAtom { file_path, heading_path, body });
        }
        Ok(out)
    }
}

/// SHA-256 hex digest (64 lowercase hex chars) of an atom body. Deterministic and
/// stable across processes AND Rust releases — mirrors `policy::audit::content_hash`.
/// Stored for future retrieval-time stale-flagging. NOT `DefaultHasher`, whose
/// algorithm is std-documented as unstable across releases (a toolchain bump would
/// re-hash every body and report spurious "changed").
fn atom_body_hash(body: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(body.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Turn an arbitrary user query into a safe FTS5 MATCH expression: extract
/// alphanumeric tokens, quote each one INDIVIDUALLY (which neutralizes FTS5
/// operators like AND/OR/NOT/NEAR and metacharacters like `* " -`), and OR them
/// for recall — BM25 then ranks by how well each atom matches. Empty when the
/// query has no alphanumeric tokens (the caller treats that as "no results").
///
/// Quoting per-token does NOT disable stemming: FTS5 tokenizes the contents of a
/// quoted string, so `porter` still applies (migrate / migration / migrations
/// share a stem — guarded by `cl_retrieve_stems_across_inflections`). Quoting each
/// token SEPARATELY (vs quoting the whole query as one phrase) also avoids
/// imposing phrase-adjacency, so word order doesn't constrain matches.
fn to_fts5_match(query: &str) -> String {
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Rough token estimate for budgeting: ~4 chars per token. Coarse and English-
/// biased — it UNDERSHOOTS for CJK/multibyte text (1 char ≉ 0.25 tokens) — but it
/// is only a budget guardrail, not a billing figure.
fn estimate_tokens(text: &str) -> i64 {
    (text.chars().count() as i64 + 3) / 4
}

#[cfg(test)]
mod tests {
    use super::{atom_body_hash, Atom};
    use crate::storage::Storage;

    fn atom(heading: &str, body: &str) -> Atom {
        Atom { heading_path: heading.into(), body: body.into() }
    }

    /// Stage-0 de-risk gate (kept): proves FTS5 is compiled into the bundled sqlite
    /// and that standalone-table INSERT, MATCH, and BM25 ranking all work. If FTS5
    /// were absent, migration 0024 (`CREATE VIRTUAL TABLE … USING fts5`) would have
    /// failed inside `Storage::memory()` and we'd never reach the asserts.
    #[tokio::test]
    async fn fts5_atoms_match_and_bm25_rank() {
        let s = Storage::memory().await.unwrap();
        s.replace_atoms_for_file(
            "bot-hq",
            "notes.md",
            &[
                atom("Gotchas", "the applied sqlx migration is immutable once shipped"),
                atom("Setup", "run cargo build then start the desktop app"),
            ],
            "2026-06-27T00:00:00+00:00",
        )
        .await
        .unwrap();

        // MATCH on a term unique to the first atom; BM25-ordered (lower = better).
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT heading_path FROM cl_atoms WHERE cl_atoms MATCH ? ORDER BY bm25(cl_atoms)",
        )
        .bind("migration")
        .fetch_all(s.pool())
        .await
        .unwrap();
        assert_eq!(rows.len(), 1, "MATCH 'migration' should hit exactly the Gotchas atom");
        assert_eq!(rows[0].0, "Gotchas");

        // A term in neither atom returns nothing (sanity).
        let none: Vec<(String,)> =
            sqlx::query_as("SELECT heading_path FROM cl_atoms WHERE cl_atoms MATCH ?")
                .bind("kubernetes")
                .fetch_all(s.pool())
                .await
                .unwrap();
        assert!(none.is_empty(), "MATCH of an absent term must return no rows");
    }

    #[tokio::test]
    async fn replace_atoms_is_idempotent_and_clears_old() {
        let s = Storage::memory().await.unwrap();
        s.replace_atoms_for_file("p", "notes.md", &[atom("A", "alpha"), atom("B", "beta")], "t1")
            .await
            .unwrap();
        // Re-replace with a different set → old atoms gone, only the new set remains.
        s.replace_atoms_for_file("p", "notes.md", &[atom("C", "gamma")], "t2")
            .await
            .unwrap();
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT heading_path FROM cl_atoms WHERE project_id='p' AND file_path='notes.md' ORDER BY heading_path",
        )
        .fetch_all(s.pool())
        .await
        .unwrap();
        assert_eq!(rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), vec!["C"]);
    }

    #[tokio::test]
    async fn delete_atoms_removes_only_that_file() {
        let s = Storage::memory().await.unwrap();
        s.replace_atoms_for_file("p", "a.md", &[atom("x", "xx")], "t").await.unwrap();
        s.replace_atoms_for_file("p", "b.md", &[atom("y", "yy")], "t").await.unwrap();
        let removed = s.delete_atoms_for_file("p", "a.md").await.unwrap();
        assert_eq!(removed, 1);
        let remaining: Vec<(String,)> =
            sqlx::query_as("SELECT file_path FROM cl_atoms ORDER BY file_path")
                .fetch_all(s.pool())
                .await
                .unwrap();
        assert_eq!(remaining.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), vec!["b.md"]);
    }

    #[test]
    fn body_hash_is_deterministic_and_64_hex() {
        // SHA-256 is stable across calls/processes (unlike DefaultHasher).
        assert_eq!(atom_body_hash("the same body"), atom_body_hash("the same body"));
        assert_ne!(atom_body_hash("body one"), atom_body_hash("body two"));
        let h = atom_body_hash("");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn cl_retrieve_ranks_by_relevance_and_scopes_by_project() {
        let s = Storage::memory().await.unwrap();
        s.replace_atoms_for_file(
            "p",
            "notes.md",
            &[
                atom("Migrations", "applied sqlx migrations are immutable"),
                atom("Frontend", "react tailwind vitest"),
            ],
            "t",
        )
        .await
        .unwrap();
        // A different project must not leak into p's results.
        s.replace_atoms_for_file("other", "x.md", &[atom("Migrations", "immutable migration here")], "t")
            .await
            .unwrap();

        let hits = s.cl_retrieve("p", "migration immutable", None, 10_000).await.unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|h| h.file_path == "notes.md"), "project-scoped");
        assert_eq!(hits[0].heading_path, "Migrations", "best match ranks first");
    }

    #[tokio::test]
    async fn cl_retrieve_sanitizes_adversarial_queries() {
        let s = Storage::memory().await.unwrap();
        s.replace_atoms_for_file("p", "n.md", &[atom("H", "alpha beta gamma")], "t").await.unwrap();
        // FTS5 operators / metacharacters must not throw — they're quoted literals.
        for q in ["alpha AND", "beta\"", "*", "gamma NEAR/2", "   ", "a OR b", "-x"] {
            s.cl_retrieve("p", q, None, 10_000).await.unwrap();
        }
        assert!(!s.cl_retrieve("p", "alpha", None, 10_000).await.unwrap().is_empty());
        // No alphanumeric tokens → no MATCH run, empty result.
        assert!(s.cl_retrieve("p", "***", None, 10_000).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn cl_retrieve_respects_token_budget() {
        let s = Storage::memory().await.unwrap();
        let big = "lorem ".repeat(20); // 120 chars → ~30 tokens each
        s.replace_atoms_for_file("p", "n.md", &[atom("A", &big), atom("B", &big), atom("C", &big)], "t")
            .await
            .unwrap();
        // Budget for ~one atom → exactly one (we always keep at least the top one).
        assert_eq!(s.cl_retrieve("p", "lorem", None, 30).await.unwrap().len(), 1);
        // Generous budget → all three.
        assert_eq!(s.cl_retrieve("p", "lorem", None, 10_000).await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn cl_retrieve_paths_filter_restricts_files() {
        let s = Storage::memory().await.unwrap();
        s.replace_atoms_for_file("p", "a.md", &[atom("H", "shared keyword")], "t").await.unwrap();
        s.replace_atoms_for_file("p", "b.md", &[atom("H", "shared keyword")], "t").await.unwrap();
        let only = vec!["a.md".to_string()];
        let hits = s.cl_retrieve("p", "keyword", Some(&only), 10_000).await.unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|h| h.file_path == "a.md"));
    }

    #[tokio::test]
    async fn cl_retrieve_pins_conventions_on_bm25_tie() {
        let s = Storage::memory().await.unwrap();
        // Identical body → identical BM25; the pin tie-break puts conventions first.
        s.replace_atoms_for_file("p", "notes.md", &[atom("H", "deploy gate keyword")], "t").await.unwrap();
        s.replace_atoms_for_file("p", "conventions.md", &[atom("H", "deploy gate keyword")], "t")
            .await
            .unwrap();
        let hits = s.cl_retrieve("p", "deploy gate keyword", None, 10_000).await.unwrap();
        assert_eq!(hits[0].file_path, "conventions.md", "pinned file wins the bm25 tie");
    }

    #[test]
    fn to_fts5_match_quotes_tokens_and_ors() {
        assert_eq!(super::to_fts5_match("alpha AND beta"), "\"alpha\" OR \"AND\" OR \"beta\"");
        assert_eq!(super::to_fts5_match("  *!*  "), "");
        assert_eq!(super::to_fts5_match("one"), "\"one\"");
    }

    #[tokio::test]
    async fn cl_retrieve_stems_across_inflections() {
        let s = Storage::memory().await.unwrap();
        s.replace_atoms_for_file("p", "n.md", &[atom("H", "applied migrations are immutable")], "t")
            .await
            .unwrap();
        // Query inflections the body does NOT contain verbatim. FTS5 tokenizes the
        // contents of a quoted string, so `porter` stemming still applies through
        // to_fts5_match's quoting: migrate/migration/migrations share a stem.
        for q in ["migrate", "migration", "MIGRATING"] {
            assert!(
                !s.cl_retrieve("p", q, None, 10_000).await.unwrap().is_empty(),
                "porter stemming should match {q:?} -> migrations (quoting keeps the tokenizer)"
            );
        }
    }
}
