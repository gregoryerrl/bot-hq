//! `roles` / `session_participants` / `participant_cursors` /
//! `participant_deliveries` — the session-focused model's persistence layer.
//!
//! Batch B3 of the redesign. Roles are user-owned templates; a participant is a
//! model plus an **invite-time snapshot** of one, so editing a role never
//! widens a live participant mid-turn. Cursors make delivery an auditable fact
//! instead of a side effect, and deliveries record what a policy withheld —
//! policies gate delivery, never persistence.
//!
//! **Schema note:** these tables ship in migration 0044, applied 2026-08-06
//! (see `docs/plans/2026-08-06-session-participants-runbook.md`). Its backfill
//! was a one-shot over the sessions that existed at apply time, so
//! [`Storage::ensure_session_roster`] is what keeps every session created since
//! from starting life with an empty roster.

use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Role {
    pub id: i64,
    pub slug: String,
    pub display_name: String,
    pub description_prompt: Option<String>,
    /// JSON array of capability slugs — grants only (0044). Kept as the raw
    /// column rather than a parsed set because `storage` has no dependency on
    /// `agents`, where [`Capability`](crate::agents::Capability) lives, and
    /// adding one for a list of strings would invert the layering. Whatever
    /// writes this must have already decided the slugs are real;
    /// [`Storage::create_role`] only guarantees the SHAPE (a JSON array of
    /// strings), never that a slug names a capability that exists.
    pub capabilities: String,
    /// `active` | `observer` | `on_demand` — see [`PARTICIPATION_MODES`].
    pub participation_mode: String,
    /// The role's default model, overridable per participant at invite
    /// (`session_participants.model_id`). Both columns ship in 0044; rc3
    /// decision D8 makes THIS one the Roles tab's model control and deletes the
    /// Agents tab rather than renaming it.
    pub default_model_id: Option<String>,
    pub builtin: bool,
    /// Archived roles are hidden from [`Storage::list_roles`] but keep their
    /// rows, their ids and their slugs — see migration 0047 for why removal
    /// cannot be a delete.
    pub archived: bool,
}

/// Everything the Roles tab may set on a role, as one value.
///
/// One struct for create AND update, because the tab edits the same form in
/// both cases and two near-identical structs would drift the moment a field is
/// added. What differs is `slug`, and only `slug` — see the field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoleDraft {
    pub display_name: String,
    /// **`None` means different things per operation, and the difference is
    /// load-bearing.**
    ///
    /// On [`Storage::create_role`]: derive the slug from `display_name`.
    ///
    /// On [`Storage::update_role`]: leave the existing slug ALONE. Re-deriving
    /// it from the display name on every edit would mean renaming the "HANDS"
    /// role to "Executor" silently moves its slug from `hands` to `executor` —
    /// and `Storage::ensure_session_roster` seeds every new session's roster
    /// with two literal `(SELECT id FROM roles WHERE slug = 'hands' / 'eyes')`
    /// subqueries. Those resolve to NULL against a renamed slug, so every
    /// session created afterwards would get a roster with `role_id IS NULL` and
    /// nothing would report an error. A rename is therefore always explicit:
    /// pass `Some`.
    ///
    /// `Some` is normalised through [`slugify`] and de-duplicated, so a caller
    /// cannot write a slug with spaces in it or collide with an existing role.
    pub slug: Option<String>,
    pub description_prompt: Option<String>,
    /// JSON array of capability slugs. Validated for SHAPE and re-serialised
    /// canonically — see [`canonical_capabilities`].
    pub capabilities: String,
    pub participation_mode: String,
    pub default_model_id: Option<String>,
}

/// The participation modes 0044's column comment defines, as data.
///
/// A guard rather than documentation because the value is compared as a STRING
/// with no CHECK constraint behind it: `next_active_participant` filters the
/// ring on `p.participation_mode == "active"`, so a role stored as `"Active"`
/// or `"actve"` produces participants that are enabled, visible in the roster,
/// counted by `all_active_voted_done` — no, not even counted, since that filters
/// on the same string — and simply never given a turn. The failure is a session
/// that looks fully staffed and never advances, with nothing to grep for.
pub const PARTICIPATION_MODES: [&str; 3] = ["active", "observer", "on_demand"];

/// The slug a role whose display name contains no ASCII alphanumerics falls
/// back to. See [`slugify`] for why that case exists at all.
const FALLBACK_SLUG: &str = "role";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participant {
    pub id: i64,
    pub session_id: String,
    pub slug: String,
    pub display_name: String,
    pub role_id: Option<i64>,
    pub model_id: Option<String>,
    pub runtime: String,
    /// Invite-time snapshot, NOT a live read of the role.
    pub capabilities: String,
    pub participation_mode: String,
    pub turn_position: i64,
    pub done_vote: bool,
    pub enabled: bool,
}

const ROLE_COLUMNS: &str = "id, slug, display_name, description_prompt, capabilities, \
     participation_mode, default_model_id, builtin, archived";

const PARTICIPANT_COLUMNS: &str = "id, session_id, slug, display_name, role_id, model_id, \
     runtime, capabilities, participation_mode, turn_position, done_vote, enabled";

fn role_from_row(r: &sqlx::sqlite::SqliteRow) -> Role {
    use sqlx::Row;
    Role {
        id: r.get("id"),
        slug: r.get("slug"),
        display_name: r.get("display_name"),
        description_prompt: r.get("description_prompt"),
        capabilities: r.get("capabilities"),
        participation_mode: r.get("participation_mode"),
        default_model_id: r.get("default_model_id"),
        builtin: r.get::<i64, _>("builtin") != 0,
        // `<> 0`, not `== 1`, for the same reason `enabled` is decoded that way:
        // the column is `INTEGER NOT NULL DEFAULT 0` with no CHECK, so 2 is a
        // storable value, and a truthiness flag has to be read as one. A row
        // storing 2 read as "not archived" would put a removed role back in the
        // picker.
        archived: r.get::<i64, _>("archived") != 0,
    }
}

fn participant_from_row(r: &sqlx::sqlite::SqliteRow) -> Participant {
    use sqlx::Row;
    Participant {
        id: r.get("id"),
        session_id: r.get("session_id"),
        slug: r.get("slug"),
        display_name: r.get("display_name"),
        role_id: r.get("role_id"),
        model_id: r.get("model_id"),
        runtime: r.get("runtime"),
        capabilities: r.get("capabilities"),
        participation_mode: r.get("participation_mode"),
        turn_position: r.get("turn_position"),
        done_vote: r.get::<i64, _>("done_vote") != 0,
        enabled: r.get::<i64, _>("enabled") != 0,
    }
}

/// The ring step, as a pure function of the rotation.
///
/// Split out of [`Storage::next_active_participant`] so the scheduling rule can
/// be exercised over rosters the database will not produce — which is the only
/// way to test what the ring does with a roster the schema once permitted.
///
/// `ring` is the active participants in `(turn_position, id)` order and nothing
/// else; observers and disabled rows are filtered out by the caller, because a
/// wake that cannot produce output is pure waste.
///
/// **The step is by POSITION IN THE RING, not by `turn_position` value.** The
/// old rule — the first member whose `turn_position` is strictly greater —
/// starves every member after the first at any shared position, and
/// `turn_position` carried no uniqueness until migration 0045.
///
/// 0045's index and this caller's filter are meant to select the same rows, and
/// keeping them in agreement is a maintenance obligation, not something either
/// side enforces on the other. They already disagreed once: the index was
/// written `WHERE enabled = 1` while [`participant_from_row`] decodes `enabled`
/// as `!= 0`, so a row storing 2 — which `INTEGER NOT NULL DEFAULT 1` with no
/// CHECK permits — was outside the index and inside the ring, and a copy of the
/// live database accepted exactly that duplicate. The predicate is `<> 0` now,
/// which is the same test the decode performs. Widening the ring to include
/// `on_demand` later is the same trap, one line away.
///
/// Stepping by ring index is what makes that a correctness question about the
/// SCHEMA rather than about scheduling: whatever set the caller's filter
/// selects, every member of it gets a turn.
fn next_in_ring<'a>(
    ring: &[&'a Participant],
    current: Option<&Participant>,
) -> Option<&'a Participant> {
    if ring.is_empty() {
        return None;
    }
    // A user message resets the cycle to the first active participant.
    let Some(current) = current else {
        return Some(ring[0]);
    };
    match ring.iter().position(|p| p.id == current.id) {
        Some(i) => Some(ring[(i + 1) % ring.len()]),
        // `current` is not in the rotation: it was disabled or demoted to
        // observer while it held the turn, so there is no place to step one
        // along FROM. Fall back to the first member sorting after where it sat,
        // in the ring's own `(turn_position, id)` order, and wrap when there is
        // none. Skipping straight to `ring[0]` instead would replay the first
        // participant's turn every time someone left mid-cycle.
        None => Some(
            ring.iter()
                .find(|p| (p.turn_position, p.id) > (current.turn_position, current.id))
                .copied()
                .unwrap_or(ring[0]),
        ),
    }
}

/// A display name reduced to a stable key: lowercase ASCII alphanumerics, with
/// every other run of characters collapsed to a single `-` and the ends
/// trimmed.
///
/// ASCII-only is a deliberate narrowing rather than an oversight. A role slug
/// is a key that gets typed and pasted — `ensure_session_roster` matches two of
/// them as SQL literals (`'hands'`, `'eyes'`) — and it seeds the participant
/// slug the rc3 mention syntax parses as `@slug`. A slug nobody can type on
/// their keyboard is worse than one that lost an accent.
///
/// The cost is named rather than hidden: `Café` slugifies to `caf`, and a name
/// written entirely in a non-Latin script slugifies to the empty string. The
/// empty case is exactly why [`FALLBACK_SLUG`] exists — `roles.slug` is
/// `NOT NULL UNIQUE`, so an empty slug would insert once and then fail every
/// subsequent time with a constraint error the user cannot act on.
fn slugify(display_name: &str) -> String {
    let mut out = String::with_capacity(display_name.len());
    for ch in display_name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            // One `-` per run of anything else, so "HANDS  //  v2" is
            // `hands-v2` rather than `hands------v2`. A leading `-` can be
            // pushed here (the string is empty, so it does not end with one);
            // the trim below removes it.
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        FALLBACK_SLUG.to_string()
    } else {
        trimmed.to_string()
    }
}

/// `base`, or the first `base-N` (N from 2) that nobody holds.
///
/// `taken` must include ARCHIVED roles: `roles.slug` is UNIQUE over the whole
/// table and 0047 did not scope that to live rows, so an archived role's slug
/// is still reserved. Handing this a live-only set would produce a slug that
/// passes here and fails at the INSERT.
///
/// Suffixes start at 2 because `base` itself is the "1": `hands`, `hands-2`,
/// `hands-3`. The search is bounded rather than unbounded, and provably
/// sufficient — `base` plus `base-2 ..= base-(N+2)` is N+2 candidates against N
/// taken slugs, so by pigeonhole at least two are free. The `expect` is
/// therefore unreachable rather than optimistic.
fn first_free_slug(base: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(base) {
        return base.to_string();
    }
    (2..=taken.len() as u64 + 2)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !taken.contains(candidate))
        .expect("N+2 candidates cannot all collide with N taken slugs")
}

/// Validate the SHAPE of a capabilities column and re-serialise it compactly.
///
/// Shape only — this checks that the value is a JSON array of strings, never
/// that a string names a capability that exists. That second check belongs
/// where [`Capability`](crate::agents::Capability) is in scope, which is not
/// here: `storage` carries no dependency on `agents`, and the Tauri command
/// layer is the Roles tab's only door.
///
/// The shape check still earns its place. `CapabilitySet::from_slugs` is a
/// `filter_map` over `Capability::parse`, so a column holding `"[]"`, `"null"`,
/// or a stray `{}` all decode to the same thing: a role with no capabilities,
/// which is a legal configuration (an observer) and reads as intentional. A
/// write that stored the wrong TYPE would be indistinguishable from a user who
/// meant to grant nothing.
///
/// Re-serialising normalises whitespace so the column holds one spelling of a
/// given list. Order and duplicates are preserved as given: dropping a repeat
/// would be a silent edit of what the caller asked to store, and order is what
/// the migration's own seed rows use.
fn canonical_capabilities(raw: &str) -> Result<String> {
    let parsed: serde_json::Value = serde_json::from_str(raw)
        .with_context(|| format!("role capabilities must be JSON, got {raw:?}"))?;
    let serde_json::Value::Array(items) = parsed else {
        anyhow::bail!("role capabilities must be a JSON array, got {raw:?}");
    };
    let slugs = items
        .iter()
        .map(|item| match item {
            serde_json::Value::String(s) => Ok(s.as_str()),
            other => Err(anyhow::anyhow!(
                "role capabilities must be an array of strings, got {other}"
            )),
        })
        .collect::<Result<Vec<&str>>>()?;
    serde_json::to_string(&slugs).context("re-serialising role capabilities")
}

/// The checks every role write runs, whichever operation is writing.
///
/// Returns the canonical capabilities so the caller cannot forget to store the
/// validated form — a `Result<()>` here would leave the raw string one typo
/// away from being the thing that gets bound.
fn validated_draft_capabilities(draft: &RoleDraft) -> Result<String> {
    if draft.display_name.trim().is_empty() {
        // The display name is what the tab lists and what a generated slug is
        // derived from. Blank, the slug falls back to `role`, `role-2`, … and
        // the picker shows a column of unlabelled rows.
        anyhow::bail!("a role needs a display name");
    }
    if !PARTICIPATION_MODES.contains(&draft.participation_mode.as_str()) {
        anyhow::bail!(
            "unknown participation mode {:?} — expected one of {}",
            draft.participation_mode,
            PARTICIPATION_MODES.join(", ")
        );
    }
    canonical_capabilities(&draft.capabilities)
}

impl Storage {
    // ---- roles ----------------------------------------------------------

    /// Live roles, archived ones excluded — what a picker should offer.
    ///
    /// The filter is `archived = 0` rather than `archived <> 1` so a row storing
    /// any other non-zero value is treated as archived, agreeing with
    /// [`role_from_row`]'s `<> 0` decode. The two must select the same rows;
    /// 0045's post-mortem is the record of what it costs when a SQL predicate
    /// and a Rust decode disagree about a truthiness column.
    pub async fn list_roles(&self) -> Result<Vec<Role>> {
        let rows = sqlx::query(&format!(
            "SELECT {ROLE_COLUMNS} FROM roles WHERE archived = 0 ORDER BY slug"
        ))
        .fetch_all(&self.pool)
        .await
        .context("listing roles")?;
        Ok(rows.iter().map(role_from_row).collect())
    }

    /// Every role, archived included. The Roles tab needs this to offer an
    /// un-archive, and a past session's `role_id` only resolves through it.
    pub async fn list_roles_including_archived(&self) -> Result<Vec<Role>> {
        let rows = sqlx::query(&format!("SELECT {ROLE_COLUMNS} FROM roles ORDER BY slug"))
            .fetch_all(&self.pool)
            .await
            .context("listing roles including archived")?;
        Ok(rows.iter().map(role_from_row).collect())
    }

    /// **Does not filter archived**, deliberately. This is a lookup by a UNIQUE
    /// key, and 0047 left the slug reserved while a role is archived precisely
    /// so the row stays reachable — hiding it here would make an archived role
    /// both unresolvable and un-recreatable under its own name.
    pub async fn role_by_slug(&self, slug: &str) -> Result<Option<Role>> {
        let row = sqlx::query(&format!("SELECT {ROLE_COLUMNS} FROM roles WHERE slug = ?"))
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .with_context(|| format!("loading role {slug}"))?;
        Ok(row.as_ref().map(role_from_row))
    }

    pub async fn role_by_id(&self, id: i64) -> Result<Option<Role>> {
        let row = sqlx::query(&format!("SELECT {ROLE_COLUMNS} FROM roles WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .with_context(|| format!("loading role {id}"))?;
        Ok(row.as_ref().map(role_from_row))
    }

    /// Every slug currently held, archived rows included, optionally excluding
    /// one row's own. See [`first_free_slug`] for why archived rows count.
    ///
    /// `exclude_id` is what makes a rename idempotent: without it, saving the
    /// `hands` role with `slug: Some("hands")` would find `hands` taken — by
    /// itself — and rename it to `hands-2` on every save.
    async fn taken_slugs(&self, exclude_id: Option<i64>) -> Result<HashSet<String>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT slug FROM roles WHERE ?1 IS NULL OR id <> ?1")
                .bind(exclude_id)
                .fetch_all(&self.pool)
                .await
                .context("reading taken role slugs")?;
        Ok(rows.into_iter().map(|(slug,)| slug).collect())
    }

    /// Create a role and return it as stored — including the slug that was
    /// allocated for it, which the caller cannot predict.
    ///
    /// `builtin` is 0 and is not a parameter: the flag means "seeded by bot-hq"
    /// (0044), and a role the user just created is not that, whatever it is
    /// named.
    ///
    /// **The slug is allocated by a SELECT and used by a later INSERT, and those
    /// are two statements.** Two creates racing on the same base name can both
    /// read the same taken-set and pick the same suffix; the second INSERT then
    /// fails on `roles.slug`'s UNIQUE index and this returns that error with the
    /// slug named in the context. That is the backstop working, not the window
    /// being closed — a caller that must not see it needs its own serialisation.
    /// Left as-is because the only caller is a desktop tab where the two writers
    /// would have to be two clicks in the same millisecond, and because a
    /// constraint error is a loud, correct failure rather than a wrong row.
    pub async fn create_role(&self, draft: &RoleDraft) -> Result<Role> {
        let capabilities = validated_draft_capabilities(draft)?;
        let base = slugify(draft.slug.as_deref().unwrap_or(&draft.display_name));
        let slug = first_free_slug(&base, &self.taken_slugs(None).await?);
        // `now_utc()` (RFC3339-Z), not the column's `datetime('now')` DEFAULT.
        // That default is what 0044 seeded `hands` and `eyes` with, so this
        // column now holds two spellings — a fact, not a claim that it does not
        // matter. RFC3339-Z is the project baseline every other write keeps
        // (`storage::time`), and the alternative is worse: `datetime('now')`
        // emits a zone-less local-looking string the frontend renders as local
        // time, which is the staleness hallucination `now_utc` exists to stop.
        let now = now_utc();
        let id = sqlx::query(
            "INSERT INTO roles \
             (slug, display_name, description_prompt, capabilities, participation_mode, \
              default_model_id, builtin, archived, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, 0, 0, ?, ?)",
        )
        .bind(&slug)
        .bind(draft.display_name.trim())
        .bind(draft.description_prompt.as_deref())
        .bind(&capabilities)
        .bind(&draft.participation_mode)
        .bind(draft.default_model_id.as_deref())
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .with_context(|| format!("creating role {slug}"))?
        .last_insert_rowid();
        self.role_by_id(id)
            .await?
            .with_context(|| format!("role {id} vanished between insert and read"))
    }

    /// Update a role in place and return it as stored.
    ///
    /// Touches exactly the five fields the Roles tab owns plus the slug. What
    /// it deliberately does NOT touch:
    ///   * `builtin` — the flag records provenance, and editing a seeded role
    ///     does not stop it having been seeded (0044: seeds are user-editable);
    ///   * `archived` — [`Storage::set_role_archived`] is the one way that
    ///     moves, so a save from a form that never rendered the flag cannot
    ///     resurrect an archived role by omission;
    ///   * `created_at`;
    ///   * every LIVE participant that was invited from this role. That is the
    ///     invite-time snapshot (0044) and it is the point: editing a role must
    ///     not widen a running participant's permissions mid-turn.
    ///
    /// Errors when `id` names no role. An UPDATE matching nothing is not an
    /// error in SQLite, so without the check a save against a role another
    /// window had just archived-and-recreated would report success and change
    /// nothing.
    pub async fn update_role(&self, id: i64, draft: &RoleDraft) -> Result<Role> {
        let capabilities = validated_draft_capabilities(draft)?;
        // `None` leaves the slug alone — see `RoleDraft::slug` for why a rename
        // has to be explicit rather than derived from the display name.
        let slug = match draft.slug.as_deref() {
            Some(requested) => {
                let base = slugify(requested);
                Some(first_free_slug(&base, &self.taken_slugs(Some(id)).await?))
            }
            None => None,
        };
        let changed = sqlx::query(
            "UPDATE roles SET \
                 slug = COALESCE(?, slug), \
                 display_name = ?, \
                 description_prompt = ?, \
                 capabilities = ?, \
                 participation_mode = ?, \
                 default_model_id = ?, \
                 updated_at = ? \
             WHERE id = ?",
        )
        .bind(slug.as_deref())
        .bind(draft.display_name.trim())
        .bind(draft.description_prompt.as_deref())
        .bind(&capabilities)
        .bind(&draft.participation_mode)
        .bind(draft.default_model_id.as_deref())
        .bind(now_utc())
        .bind(id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("updating role {id}"))?
        .rows_affected();
        if changed == 0 {
            anyhow::bail!("role {id} does not exist");
        }
        self.role_by_id(id)
            .await?
            .with_context(|| format!("role {id} vanished between update and read"))
    }

    /// Archive or un-archive a role (decision D8: removal is archival).
    ///
    /// Takes the target state rather than being a one-way `archive_role`, so
    /// the same call restores. Nothing else about the row moves — the slug
    /// stays reserved (0047) and every participant that carries this `role_id`
    /// keeps resolving it.
    ///
    /// Errors when `id` names no role, for the same reason
    /// [`Storage::update_role`] does: a no-match UPDATE is silent success in
    /// SQLite, and "the role you archived is still in the list" is exactly the
    /// bug that would produce.
    pub async fn set_role_archived(&self, id: i64, archived: bool) -> Result<()> {
        let changed = sqlx::query("UPDATE roles SET archived = ?, updated_at = ? WHERE id = ?")
            .bind(i64::from(archived))
            .bind(now_utc())
            .bind(id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("archiving role {id}"))?
            .rows_affected();
        if changed == 0 {
            anyhow::bail!("role {id} does not exist");
        }
        Ok(())
    }

    // ---- participants ---------------------------------------------------

    /// Invite a participant, snapshotting the role's capabilities and
    /// participation mode. Passing them explicitly (rather than reading the
    /// role here) keeps the snapshot honest: the caller composes what this
    /// participant actually runs with, including any session-policy ceiling.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_participant(
        &self,
        session_id: &str,
        slug: &str,
        display_name: &str,
        role_id: Option<i64>,
        model_id: Option<&str>,
        capabilities: &str,
        participation_mode: &str,
        turn_position: i64,
    ) -> Result<i64> {
        let id = sqlx::query(
            "INSERT INTO session_participants \
             (session_id, slug, display_name, role_id, model_id, capabilities, \
              participation_mode, turn_position) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(slug)
        .bind(display_name)
        .bind(role_id)
        .bind(model_id)
        .bind(capabilities)
        .bind(participation_mode)
        .bind(turn_position)
        .execute(&self.pool)
        .await
        .with_context(|| format!("inserting participant {slug} into {session_id}"))?
        .last_insert_rowid();
        // Every participant reads the channel, so every participant has a cursor.
        sqlx::query("INSERT INTO participant_cursors (participant_id) VALUES (?)")
            .bind(id)
            .execute(&self.pool)
            .await
            .context("seeding participant cursor")?;
        Ok(id)
    }

    /// Roster in turn order — the order the sequencer advances through and the
    /// order the UI renders.
    pub async fn participants_for_session(&self, session_id: &str) -> Result<Vec<Participant>> {
        let rows = sqlx::query(&format!(
            "SELECT {PARTICIPANT_COLUMNS} FROM session_participants \
             WHERE session_id = ? ORDER BY turn_position, id"
        ))
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("listing participants for {session_id}"))?;
        Ok(rows.iter().map(participant_from_row).collect())
    }

    pub async fn participant_by_slug(
        &self,
        session_id: &str,
        slug: &str,
    ) -> Result<Option<Participant>> {
        let row = sqlx::query(&format!(
            "SELECT {PARTICIPANT_COLUMNS} FROM session_participants \
             WHERE session_id = ? AND slug = ?"
        ))
        .bind(session_id)
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .context("loading participant")?;
        Ok(row.as_ref().map(participant_from_row))
    }

    /// Seed the default roster for a session that has none, returning how many
    /// participants were inserted (0 on the common path).
    ///
    /// 0044 backfilled `session_participants` from the paired `brian_*`/`rain_*`
    /// columns as a **one-shot over the rows that existed when it applied**.
    /// Nothing then created participants for a NEW session, so every message it
    /// wrote resolved `participant_id` to NULL forever — the dual-write in
    /// `insert_message` looks up the roster by slug. Called pre-spawn from
    /// `ensure_session_started`, which every creation path funnels through, so
    /// this both seeds new sessions and heals any left rosterless by that
    /// window.
    ///
    /// The two INSERTs mirror 0044's backfill statement-for-statement, scoped
    /// to one session: a seeded roster is then structurally identical to a
    /// backfilled one, which is what stops the two populations drifting.
    /// `OR IGNORE` rides `UNIQUE (session_id, slug)` for idempotence, so a
    /// healthy respawn pays two no-op inserts and nothing else.
    pub async fn ensure_session_roster(&self, session_id: &str) -> Result<u64> {
        let hands = sqlx::query(
            "INSERT OR IGNORE INTO session_participants \
             (session_id, slug, display_name, role_id, model_id, effort, ultracode, \
              claude_session_id, capabilities, participation_mode, turn_position, joined_at) \
             SELECT s.id, 'brian', 'Brian', \
                    (SELECT id FROM roles WHERE slug = 'hands'), \
                    COALESCE(s.brian_model_id, s.brian_model_at_spawn), \
                    s.brian_effort, s.brian_ultracode, s.brian_claude_session_id, \
                    (SELECT capabilities FROM roles WHERE slug = 'hands'), \
                    (SELECT participation_mode FROM roles WHERE slug = 'hands'), \
                    0, s.created_at \
             FROM sessions s WHERE s.id = ?",
        )
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("seeding HANDS participant for {session_id}"))?
        .rows_affected();
        // turn_position 1: the reviewer always sees the executor's work before
        // responding. A solo session keeps the row and disables it, exactly as
        // 0044 did for the 12 solo sessions it backfilled.
        let eyes = sqlx::query(
            "INSERT OR IGNORE INTO session_participants \
             (session_id, slug, display_name, role_id, model_id, effort, ultracode, \
              claude_session_id, capabilities, participation_mode, turn_position, \
              enabled, joined_at) \
             SELECT s.id, 'rain', 'Rain', \
                    (SELECT id FROM roles WHERE slug = 'eyes'), \
                    COALESCE(s.rain_model_id, s.rain_model_at_spawn), \
                    s.rain_effort, s.rain_ultracode, s.rain_claude_session_id, \
                    (SELECT capabilities FROM roles WHERE slug = 'eyes'), \
                    (SELECT participation_mode FROM roles WHERE slug = 'eyes'), \
                    1, s.rain_enabled, s.created_at \
             FROM sessions s WHERE s.id = ?",
        )
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("seeding EYES participant for {session_id}"))?
        .rows_affected();

        let inserted = hands + eyes;
        if inserted == 0 {
            return Ok(0);
        }
        // Every participant reads the channel, so every participant has a
        // cursor from birth — same invariant `insert_participant` holds.
        sqlx::query(
            "INSERT OR IGNORE INTO participant_cursors (participant_id) \
             SELECT id FROM session_participants WHERE session_id = ?",
        )
        .bind(session_id)
        .execute(&self.pool)
        .await
        .context("seeding cursors for the seeded roster")?;
        // Repair what the rosterless window wrote. Only reachable when this
        // call actually inserted, so a healthy spawn never runs it. Scoped to
        // `origin = 'participant'`: user/system rows have no participant by
        // design, and pre-0044 rows were already mapped by the migration.
        sqlx::query(
            "UPDATE messages SET participant_id = ( \
                 SELECT p.id FROM session_participants p \
                 WHERE p.session_id = messages.session_id AND p.slug = messages.author) \
             WHERE session_id = ? AND participant_id IS NULL AND origin = 'participant'",
        )
        .bind(session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("repairing unmapped messages in {session_id}"))?;
        Ok(inserted)
    }

    // ---- turn cycle -----------------------------------------------------

    /// The participant after `current` in the ring, skipping anyone not in the
    /// rotation. Observers are SKIPPED rather than given a no-op turn: a wake
    /// that cannot produce output is pure waste.
    ///
    /// `None` means nobody is active, and that IMPLIES
    /// [`Storage::all_active_voted_done`] is `true` — one way only. The converse
    /// is false and must not be assumed: a rotation where every active
    /// participant has voted done is also `true` there while this still returns
    /// `Some`, because the ring is unchanged by how anyone voted. So
    /// `next_active_participant(..).is_none()` is NOT a consensus test; it never
    /// fires while any active participant exists. Ask
    /// [`Storage::all_active_voted_done`] for consensus and use this only to
    /// find whose turn is next.
    ///
    /// `current` is the participant that just held the turn, not its position:
    /// the ring steps by place IN the rotation, so it needs to know WHICH row
    /// held the turn rather than only where that row sat. `None` resets the
    /// cycle to the front, which is what a user message does.
    pub async fn next_active_participant(
        &self,
        session_id: &str,
        current: Option<&Participant>,
    ) -> Result<Option<Participant>> {
        let roster = self.participants_for_session(session_id).await?;
        // `participants_for_session` orders by `(turn_position, id)`, so this
        // filter preserves ring order — which is what [`next_in_ring`] assumes.
        let ring: Vec<&Participant> = roster
            .iter()
            .filter(|p| p.enabled && p.participation_mode == "active")
            .collect();
        Ok(next_in_ring(&ring, current).cloned())
    }

    pub async fn set_done_vote(&self, participant_id: i64, done: bool) -> Result<()> {
        sqlx::query("UPDATE session_participants SET done_vote = ? WHERE id = ?")
            .bind(i64::from(done))
            .bind(participant_id)
            .execute(&self.pool)
            .await
            .context("setting done vote")?;
        Ok(())
    }

    /// Any substantive output resets EVERY vote — a stale "done" must not
    /// accumulate into a false arrival.
    pub async fn clear_done_votes(&self, session_id: &str) -> Result<()> {
        sqlx::query("UPDATE session_participants SET done_vote = 0 WHERE session_id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .context("clearing done votes")?;
        Ok(())
    }

    /// Consensus halt: every ACTIVE participant has declared done.
    ///
    /// **An empty rotation is done.** Vacuously — there is no participant left
    /// who has not voted — and the pair with
    /// [`Storage::next_active_participant`] is why it is written that way
    /// rather than as `false`. That returns `None` when nobody is active, which
    /// is "no turn to hand out", not "done"; this returned `false`, which is
    /// "not done". Neither answer stopped a loop, and neither gave it anything
    /// to do, so a sequencer reading the two spun.
    ///
    /// **This is the halt test, and it is the only one.** An empty rotation
    /// makes both answers agree, but the two are NOT the same condition: with
    /// every active participant voted done this is `true` while
    /// `next_active_participant` still returns `Some` — the ring does not care
    /// how anyone voted. The implication runs one way (no actives ⟹ done) and
    /// the operational rule follows from THIS side: ask consensus, halt on it,
    /// and take a turn only if it says no. Reading `is_none()` as "done" instead
    /// would never halt a session that has participants in it.
    ///
    /// The states that reach the empty-rotation case: an all-observer or
    /// all-`on_demand` roster, a roster whose every active participant has been
    /// disabled (what disabling the last agent produces), and a session with no
    /// roster yet, since `ensure_session_roster` only runs pre-spawn.
    pub async fn all_active_voted_done(&self, session_id: &str) -> Result<bool> {
        let roster = self.participants_for_session(session_id).await?;
        Ok(roster
            .iter()
            .filter(|p| p.enabled && p.participation_mode == "active")
            .all(|p| p.done_vote))
    }

    // ---- channel cursors + deliveries -----------------------------------

    pub async fn cursor_for(&self, participant_id: i64) -> Result<i64> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT last_read_message_id FROM participant_cursors WHERE participant_id = ?",
        )
        .bind(participant_id)
        .fetch_optional(&self.pool)
        .await
        .context("reading cursor")?;
        Ok(row.map(|r| r.0).unwrap_or(0))
    }

    /// Record what a participant was handed and move its cursor past the batch,
    /// in ONE transaction.
    ///
    /// **The pairing is the point.** These were two public methods, and a
    /// delivery needs both: a `participant_deliveries` row per message so
    /// "what did participant X receive?" has an answer, and a cursor move so
    /// the same rows are not offered again. Called separately they are two
    /// commits, and a crash between them leaves a cursor past rows with no
    /// record behind them — the module's own claim, quietly false, and
    /// unrecoverable because the cursor never rewinds. One `BEGIN`/`COMMIT`
    /// around both is what closes that window, and taking the whole batch is
    /// what stops a caller re-opening it by hand.
    ///
    /// `withheld_reason = None` means delivered. **A withheld message still
    /// gets a row and still advances the cursor**: policies gate delivery,
    /// never persistence, and leaving the cursor behind would re-offer the row
    /// on every subsequent turn instead of recording once that it was withheld.
    /// [`Storage::withheld_for_participant`] is where that record is read back.
    ///
    /// The cursor lands on the HIGHEST message id in the batch, derived here
    /// rather than passed, so the cursor and the records cannot disagree about
    /// how far the batch reached. It still only ever moves forward: a late
    /// delivery for an already-passed row is recorded without rewinding.
    /// An empty batch is a no-op.
    ///
    /// **Errors when the participant has no cursor row.** That is not
    /// defensiveness for its own sake: the cursor half is an UPDATE matched on
    /// `participant_id`, and an UPDATE matching nothing is not an error in
    /// SQLite, so the missing-cursor case would otherwise record deliveries,
    /// leave the cursor at 0, re-offer the same batch every turn, and report
    /// success throughout. The transaction alone does not cover it — there is
    /// nothing to roll back until something fails.
    pub async fn commit_delivery(
        &self,
        participant_id: i64,
        deliveries: &[(i64, Option<&str>)],
    ) -> Result<()> {
        if deliveries.is_empty() {
            return Ok(());
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .context("opening the delivery transaction")?;
        for (message_id, withheld_reason) in deliveries {
            // `OR IGNORE` on `UNIQUE (participant_id, message_id)`: re-recording
            // a delivery is idempotent, not an error, so a retried turn does not
            // fail the whole batch.
            sqlx::query(
                "INSERT OR IGNORE INTO participant_deliveries \
                 (participant_id, message_id, delivered_at, withheld_reason) \
                 VALUES (?, ?, CASE WHEN ?3 IS NULL THEN datetime('now') ELSE NULL END, ?3)",
            )
            .bind(participant_id)
            .bind(message_id)
            .bind(*withheld_reason)
            .execute(&mut *tx)
            .await
            .context("recording delivery")?;
        }
        let high = deliveries
            .iter()
            .map(|(message_id, _)| *message_id)
            .max()
            .expect("a non-empty batch has a highest id");
        // Cursors only ever move FORWARD. A rewind would re-deliver messages an
        // agent has already acted on — the staleness class this redesign
        // removes — so the MAX() is in the statement, not in the caller.
        let moved = sqlx::query(
            "UPDATE participant_cursors \
             SET last_read_message_id = MAX(last_read_message_id, ?), \
                 updated_at = datetime('now') \
             WHERE participant_id = ?",
        )
        .bind(high)
        .bind(participant_id)
        .execute(&mut *tx)
        .await
        .context("advancing cursor")?
        .rows_affected();
        // An UPDATE that matches nothing is not an error in SQLite, so without
        // this check a participant with no cursor row would have its deliveries
        // recorded, its cursor stay at 0, and the same batch re-offered every
        // turn — with this method reporting success each time. Failing here
        // rolls the batch back with it, which is the honest outcome: a delivery
        // whose cursor cannot move has not happened.
        if moved != 1 {
            anyhow::bail!(
                "participant {participant_id} has no cursor row, so a delivery \
                 of {} message(s) could not be recorded",
                deliveries.len()
            );
        }
        tx.commit().await.context("committing the delivery")?;
        Ok(())
    }

    /// What a participant was NOT shown, and why. The query that makes
    /// "what did participant X actually receive?" answerable.
    pub async fn withheld_for_participant(
        &self,
        participant_id: i64,
    ) -> Result<Vec<(i64, String)>> {
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT message_id, withheld_reason FROM participant_deliveries \
             WHERE participant_id = ? AND withheld_reason IS NOT NULL \
             ORDER BY message_id",
        )
        .bind(participant_id)
        .fetch_all(&self.pool)
        .await
        .context("listing withheld deliveries")?;
        Ok(rows)
    }
}

/// The decoration a message carries on its way to a participant: the IPAV
/// phase tag, the blocking-findings banner, a host-authored system prefix.
///
/// These are exactly the strings that used to be concatenated onto the wire
/// AFTER the row was written, which is why "what did the agent actually read?"
/// had no answer. Stored as JSON in `messages.envelope` (migration 0044) and
/// turned back into bytes by [`render_wire`] at delivery.
///
/// **Metadata, not a pre-rendered prefix.** The column could have held the
/// finished string, and that would be one fewer moving part — but then the
/// wire would be decided at post time by whoever happened to insert the row,
/// and the same phase tag would be spelled a different way per call site. A
/// struct means the fields are queryable, and *delivered == recorded* holds
/// because rendering is deterministic from `(body, envelope)`: given the row you
/// can rebuild the exact bytes, whenever you ask.
///
/// "Whenever" is the part that earns the struct. [`PersistedMessage::from_row`]
/// re-renders a row the sequencer reads back, under the envelope it should go
/// out with — a pre-rendered column would replay the phase tag the row was
/// POSTED under and there would be no way to tell the two apart.
///
/// Not yet *displayed*, though. The chat pane reads [`Message`], which carries
/// no envelope, so the UI still shows the body alone; rendering the decoration
/// beside it is what the queryable fields make possible, not what happens today.
///
/// Fields are private, but be precise about what that buys — it is NOT the
/// unforgeability [`PersistedMessage`] has. Every field has a `pub` builder and
/// `Deserialize` is derived on top of that, so an arbitrary `Envelope` is
/// constructible from anywhere, and `core` does build them outright. Privacy
/// buys the smaller thing: [`render_wire`] is the only code that interprets
/// these fields, so the on-disk JSON shape stays free to change.
///
/// Nor is an `Envelope` a permission. Attached to a row it is recorded and
/// re-renderable; handed straight to [`render_wire`] it is plain string
/// building, which is what `core::broadcast::peer_forward_message` still does.
/// The permission is the receipt below, and only [`Storage::post_to_channel`]
/// mints one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    /// IPAV phase NAME (`Investigate` / `Plan` / `Apply` / `Verify`), not the
    /// `IpavPhase` enum — `storage` has no dependency on `core` and adding one
    /// for a four-word string would invert the layering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    phase: Option<String>,
    /// Unresolved EYES blocking findings; `0` renders nothing.
    #[serde(default, skip_serializing_if = "is_zero")]
    open_blocking: usize,
    /// A host note that rides in front of the body — e.g. the post-cancel
    /// reconciliation directive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    system_prefix: Option<String>,
    /// This turn called `peer_ack` but carried substantive text, so the ack was
    /// overridden and the turn posted anyway (router inventory #9).
    ///
    /// The router expressed this by splicing a sentence onto the front of the
    /// body — the invisible string mutation this column exists to replace. As a
    /// field it is renderable, queryable, and cannot be mistaken for something
    /// the agent wrote.
    #[serde(default, skip_serializing_if = "is_false")]
    peer_ack_override: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl Envelope {
    pub fn phase(phase: impl Into<String>) -> Self {
        Self {
            phase: Some(phase.into()),
            ..Self::default()
        }
    }

    pub fn with_open_blocking(mut self, open_blocking: usize) -> Self {
        self.open_blocking = open_blocking;
        self
    }

    pub fn with_system_prefix(mut self, system_prefix: impl Into<String>) -> Self {
        self.system_prefix = Some(system_prefix.into());
        self
    }

    pub fn with_peer_ack_override(mut self, peer_ack_override: bool) -> Self {
        self.peer_ack_override = peer_ack_override;
        self
    }
}

/// Render `(envelope, body)` into the bytes a participant reads.
///
/// The whole point of the receipt is that this is the ONLY thing standing
/// between a row and an agent's stdin, so it is a free function of its two
/// arguments: no clock, no database, no session state. Given the same row it
/// produces the same wire forever, which is what lets the chat pane claim it is
/// showing what the agent read.
///
/// Order is load-bearing and reproduces what the pre-B5 call sites built by
/// hand: phase tag, then findings banner, then system prefix, then the body.
/// The prefix sits closest to the body because those sites concatenated it onto
/// the body FIRST and wrapped the pair in the phase envelope afterwards.
pub fn render_wire(envelope: Option<&Envelope>, body: &str) -> String {
    let Some(envelope) = envelope else {
        return body.to_string();
    };
    // Body plus headroom for the decoration, whose largest component is the
    // ~130-char findings banner. A miss costs one realloc, never correctness.
    let mut wire = String::with_capacity(body.len() + 192);
    if let Some(phase) = &envelope.phase {
        wire.push_str("[PHASE: ");
        wire.push_str(phase);
        wire.push_str("]\n");
    }
    if envelope.open_blocking > 0 {
        // The banner rides every turn until the findings are dispositioned —
        // salience, not a gate (post-mortem §5.2).
        wire.push_str(&format!(
            "⚠ {} unresolved EYES blocking finding(s) — run check_open_findings and \
             disposition each (fix/rebut) before you commit.\n",
            envelope.open_blocking
        ));
    }
    if let Some(prefix) = &envelope.system_prefix {
        wire.push_str(prefix);
        wire.push('\n');
    }
    // Closest to the body, and after `system_prefix`, because the router built
    // it that way: it spliced this sentence directly onto the trimmed body and
    // everything else wrapped the pair. Same wording, so a session that reads
    // both sides of task 14 sees no change.
    if envelope.peer_ack_override {
        wire.push_str(
            "[peer_ack overridden — this turn carried substantive text, so it was \
             forwarded anyway]\n",
        );
    }
    wire.push_str(body);
    wire
}

/// One row of the session channel, as a participant reads it.
///
/// The fields are public because reading them is the point — `core` reads
/// `.content`, `.envelope`, `.origin` and `.id` — but the struct cannot be
/// BUILT outside this module, and that is load-bearing rather than tidiness.
/// [`PersistedMessage::from_row`] turns one of these into a receipt, so a
/// `ChannelMessage` anyone could assemble would be a receipt anyone could
/// assemble, and the receipt's whole claim is that a row exists behind it.
///
/// `_from_table` is what enforces it. Privacy on the module is not enough:
/// `mod participants` is private to `storage`, which means every descendant of
/// `storage` — about ten files — can name this type and write a literal. One
/// private zero-sized field makes that literal `E0451` everywhere except here.
/// `#[non_exhaustive]` would NOT do it: that only restricts other crates, and
/// the forge that matters is in-crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelMessage {
    pub id: i64,
    pub session_id: String,
    pub participant_id: Option<i64>,
    pub origin: String,
    pub kind: String,
    pub content: String,
    pub envelope: Option<Envelope>,
    pub created_at: String,
    /// See the type doc: zero-sized, private, and the only reason "this value
    /// came out of `messages`" is enforced rather than merely true today.
    _from_table: (),
}

/// One bounded read of the channel.
///
/// The bound is the point, and so is [`ChannelPage::more`]. A read that just
/// truncated would make "the participant is caught up" and "the participant hit
/// the cap" the same value, and the second silently drops the rest of the
/// session — which is worse than the unbounded read it replaced, because it
/// looks fine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelPage {
    /// Oldest first, at most the `limit` the read was given.
    pub rows: Vec<ChannelMessage>,
    /// `true` when the limit cut the read short: rows exist after
    /// `rows.last()`. `false` means this is the whole remainder — that's all.
    pub more: bool,
}

/// How many channel rows a participant is handed in one turn.
///
/// A cap on ROWS, not on bytes, and the difference matters: the largest single
/// row in the live database is ~2 MB, so one row can outweigh this whole batch.
/// What it bounds is the unbounded case — a participant that has never read.
/// The live channel averages 538 rows per session and reaches 3,585, at ~1.9 KB
/// per row, so an uncapped backlog read was up to ~6.7 MB in one `Vec` and then
/// onto one agent's stdin. 266 of 382 sessions hold more than this many rows, so
/// [`ChannelPage::more`] is a normal outcome and not an edge case.
///
/// Not a context-window computation. `storage` does not know which model is
/// reading, and a row budget cannot stand in for a token budget; this is a
/// transport bound, and whatever trims a turn's context to a model belongs
/// where the model is known.
pub const UNREAD_BATCH_LIMIT: i64 = 200;

const CHANNEL_COLUMNS: &str =
    "id, session_id, participant_id, origin, kind, content, envelope, created_at";

fn channel_from_row(r: &sqlx::sqlite::SqliteRow) -> ChannelMessage {
    use sqlx::Row;
    let envelope: Option<String> = r.get("envelope");
    ChannelMessage {
        id: r.get("id"),
        session_id: r.get("session_id"),
        participant_id: r.get("participant_id"),
        origin: r.get("origin"),
        kind: r.get("kind"),
        content: r.get("content"),
        // A row whose envelope will not parse is still a row: dropping the
        // whole message because its decoration is malformed would lose the
        // body, which is the part that matters. Logged rather than silent —
        // an unparseable envelope means a reader and a writer disagree.
        envelope: envelope.and_then(|json| match serde_json::from_str(&json) {
            Ok(e) => Some(e),
            Err(e) => {
                tracing::warn!(error = %e, envelope = %json, "unparseable message envelope");
                None
            }
        }),
        created_at: r.get("created_at"),
        // The one place this may be written: the value is coming off a
        // `SELECT`, which is exactly what the field asserts.
        _from_table: (),
    }
}

/// A receipt for one channel row, minted by — and only by — the INSERT in
/// [`Storage::post_to_channel`].
///
/// **This is the permission to write to a participant's stdin.** B5 Task 2 made
/// the delivery path take a `&PersistedMessage`, so the host paths that used to
/// push a bare string at an agent now have to post the row first and hand over
/// the receipt. What an agent reads is [`PersistedMessage::wire`] — body plus
/// rendered envelope — and nothing else, so the wire is RECONSTRUCTIBLE from the
/// row: `render_wire(row.envelope, row.content)` reproduces it byte for byte.
///
/// That is recorded == delivered, and it is as far as the claim goes today. The
/// chat pane does not yet show it: `messages_for_session` returns a [`Message`],
/// which has no `envelope` field, so nothing in the Tauri command, the event
/// payload or `bindings.ts` carries the decoration. Displaying what the agent
/// read is a UI that reads the column — tracked separately — not something this
/// type already delivers.
///
/// One string wire survives Task 2: the peer forward in
/// `core::broadcast::peer_forward_message`. The TEXT it carries is on record —
/// an agent's own output, persisted chunk by chunk, or for the host-authored
/// provider-limit notice its own `system` row, posted beside the forward. What
/// is not is the decoration the router wraps it in at forward time. See that
/// function for why gating it is the turn sequencer's job, not this one's.
///
/// The value cannot be forged from outside. There are exactly TWO construction
/// sites — immediately downstream of that INSERT, and
/// [`PersistedMessage::from_row`] for a row read back out — and neither is
/// reachable beyond this file. The receipt's own fields are private, which
/// blocks a literal; and `from_row`'s argument carries a private field of its
/// own, which blocks assembling the row to feed it. The second gate is not a
/// formality: module privacy alone would have left every file under
/// `src/storage/` able to forge, which is how an earlier version of this
/// paragraph was wrong. That makes this file, `mod tests` included, the trusted
/// boundary. Keeping it to those two is a maintainer's job, not something the
/// compiler checks: a helper added here later could mint a receipt with no row
/// behind it. The claim now
/// covers every write to `messages`, not just this method: B5 Task 1b made
/// [`Storage::insert_message`] — the second live insert path, and the one the
/// duo pump uses on every chunk — a thin wrapper over `post_to_channel`, so
/// there is one INSERT and every row that reaches the table has a receipt
/// behind it.
///
/// `Clone` is deliberate. Fan-out hands one row to N agents by reference, so a
/// clone is never what reaches the wire; consuming by move would instead push
/// callers into re-posting the same text once per recipient.
///
/// Two things enforce that, neither of them a convention. The private fields
/// reject a struct literal (`E0451`, below), and `from_row` — the other way in —
/// takes a `ChannelMessage`, which carries a private field of its own, so it
/// cannot be fed a row that never came out of the table. Forging is rejected:
///
/// ```compile_fail
/// use bot_hq::storage::PersistedMessage;
///
/// // Nothing was inserted, so there is no row to be proof of. The struct
/// // literal cannot name the fields: they are private to
/// // `storage::participants`, which is the whole point of the type.
/// let forged = PersistedMessage {
///     message_id: 1,
///     session_id: "s1".into(),
///     body: "never persisted".to_string(),
///     envelope: None,
/// };
/// ```
///
/// Two ways that block can rot. It asserts only THAT the snippet fails, never
/// why — stable rustdoc ignores a `compile_fail,E0451` error code (verified: a
/// deliberately wrong code still passes), so re-check the reason by deleting
/// `compile_fail` and reading the real error. And a rename or a moved `pub use`
/// would leave it passing for the wrong reason; the companion doctest below
/// fails loudly if the path stops resolving.
///
/// ```
/// use bot_hq::storage::PersistedMessage;
/// fn _wire(_receipt: &PersistedMessage) {}
/// ```
#[derive(Debug, Clone)]
pub struct PersistedMessage {
    message_id: i64,
    /// The channel this receipt is valid for. A receipt without a scope is
    /// forgeable ACROSS sessions:
    /// `session_a_handle.send_to_all(receipt_from_session_b)` would wire another
    /// session's text into these agents, with the row sitting in the wrong
    /// channel — the exact class of bug this type exists to rule out.
    ///
    /// Carrying the id is only half of it, and for one batch it was the only
    /// half: nothing compared it, so the call above still compiled and ran.
    /// `SessionHandle::send_to_all` now rejects a mismatch, which is the earliest
    /// point that knows both ids. The field buys detection; that check is what
    /// makes it prevention.
    ///
    /// `Arc<str>` rather than `String` to match `DuoConfig::session_id` and the
    /// `MessagePersisted` / `BatchEmitter` threading this will flow into.
    session_id: Arc<str>,
    body: String,
    envelope: Option<Envelope>,
}

impl PersistedMessage {
    pub fn message_id(&self) -> i64 {
        self.message_id
    }

    /// The channel this receipt authorizes delivery into — see the field.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    pub fn envelope(&self) -> Option<&Envelope> {
        self.envelope.as_ref()
    }

    /// The bytes this row puts on a participant's stdin.
    ///
    /// Kept as a method on the receipt rather than left to each delivery site
    /// so there is one answer to "what did the agent read": every caller of
    /// [`render_wire`] on the delivery path goes through here, and the
    /// arguments cannot drift from the row they came from.
    pub fn wire(&self) -> String {
        render_wire(self.envelope.as_ref(), &self.body)
    }

    /// A receipt for a row READ BACK from `messages`.
    ///
    /// The second construction site, and deliberate rather than a leak. A row
    /// that came out of the table is proof of a row exactly as much as one that
    /// just went in — the type's claim is "this text is on record", and a
    /// `SELECT` establishes that at least as well as an `INSERT`.
    ///
    /// Be exact about what stops this being a forge-anything hatch. It is NOT
    /// module privacy: `mod participants` is private to `storage`, not to this
    /// file, so every descendant of `storage` — about ten files — can name
    /// [`ChannelMessage`]. An earlier draft of this comment claimed otherwise
    /// and a reviewer disproved it by compiling a fake row in
    /// `storage::messages` and getting a receipt out of it.
    ///
    /// What stops it is the private `_from_table` field on `ChannelMessage`: a
    /// struct literal outside `participants.rs` is `E0451`, so the only rows
    /// that exist came off a `SELECT`. `pub(crate)` on this method keeps it in
    /// the crate besides, matching `ParticipantInput::send_unrouted`. Callers in
    /// `core` can pass a row they were HANDED and cannot invent one.
    ///
    /// That gates PROVENANCE, not immutability, and the difference is worth
    /// keeping straight. `ChannelMessage`'s other fields stay public because
    /// `core` reads them, so code holding a row it owns can still edit `.content`
    /// before calling this. What is ruled out is a row that was never in the
    /// table — the case where no INSERT ever happened. Editing one you were
    /// handed is a visibly different act, in-crate, and not what the receipt is
    /// defending against.
    ///
    /// It exists because without it there is no way to deliver history. The turn
    /// sequencer hands each participant its backlog from
    /// [`Storage::unread_for_participant`], which returns [`ChannelMessage`]s,
    /// and every row written before a restart is only ever available that way.
    /// With `deliver` taking a receipt and receipts minted only by the INSERT,
    /// the sequencer's only exits would have been `send_unrouted` — dissolving
    /// the gate this batch built — or widening `deliver` back to strings.
    ///
    /// This is also what cashes the struct-over-string choice above. The
    /// sequencer reads a `ChannelMessage`, and [`render_wire`] rebuilds the wire
    /// from `(body, envelope)`; had the column held a pre-rendered prefix, a
    /// replayed row would carry the phase tag it was posted under with no way to
    /// tell that from the one it should be read under.
    ///
    /// Costs a clone of the body: the caller owns the row and may still need it,
    /// and this is the read path, not the per-chunk write path the module doc
    /// guards against copying on.
    // The `#[allow(dead_code)]` this carried while it waited for a caller is
    // gone, as its own note said it would be: `core::sequencer::deliver_backlog`
    // is that caller, and the method is load-bearing rather than speculative.
    pub(crate) fn from_row(row: &ChannelMessage) -> Self {
        Self {
            message_id: row.id,
            session_id: Arc::from(row.session_id.as_str()),
            body: row.content.clone(),
            envelope: row.envelope.clone(),
        }
    }
}

impl Storage {
    /// Post to the session channel — the write half of "the channel is the
    /// transport". Every wire into a participant goes through here, including
    /// the host-authored injections (`origin = "system"`) that used to be
    /// written straight to stdin and never recorded at all.
    ///
    /// Returns a [`PersistedMessage`] rather than a bare id. The receipt is the
    /// permission to wire the text: the delivery path takes one, so no caller
    /// can hand it a string that never became a row.
    ///
    /// `envelope` is the decoration the wire will carry, and it is taken HERE
    /// rather than applied at the send because the row has to record what the
    /// agent will read. Two call sites had to be reordered to supply it (the
    /// user broadcast's findings count, the tray fallback's phase); that
    /// reordering is the price of the invariant, not an accident of it.
    ///
    /// Writes the legacy `author` column too, so readers that have not migrated
    /// yet keep working (migration revision 3). `system` rows are stored as
    /// `author = 'user'` because that is exactly how today's system notices are
    /// already persisted — legacy readers must not encounter a new author value.
    pub async fn post_to_channel(
        &self,
        session_id: impl Into<Arc<str>>,
        origin: &str,
        participant_slug: Option<&str>,
        kind: &str,
        content: impl Into<String>,
        envelope: Option<Envelope>,
    ) -> Result<PersistedMessage> {
        // `impl Into<Arc<str>>` so a caller already holding one — Task 3's do,
        // from `DuoConfig::session_id` — passes a refcount bump instead of
        // deref-ing to `&str` and re-allocating. To be clear about the size of
        // the win: a ~36-byte copy next to a SQLite INSERT is noise. This is
        // consistency with the ownership argument above, not a perf claim, and
        // it is one line now versus a signature change once six call sites
        // exist.
        let session_id: Arc<str> = session_id.into();
        // Both taken by value and MOVED into the receipt at the bottom. This
        // fires per Text / ToolUse / ToolResult chunk and a tool result can
        // carry a whole file, so copying the body into the receipt would be a
        // full-body heap copy per post. `Arc<str>` would not help —
        // `Arc::from(&str)` allocates and copies exactly as `to_string()` does,
        // and delivery is by reference, so nothing ever clones the body.
        //
        // `content` is `impl Into<String>` so `&str` callers stay ergonomic and
        // `String` callers pay nothing. `envelope` is a concrete
        // `Option<Envelope>`, not a generic: a bare `None` could not be
        // inferred through `Option<impl Into<_>>` (E0283), which would force a
        // turbofish at every envelope-less call site.
        let content: String = content.into();
        // Serialised once, here, so the JSON in the column is always what the
        // receipt renders from. `Envelope` is strings and a usize, so this
        // cannot fail in practice; it is still propagated rather than unwrapped,
        // because losing the decoration silently would mean the row records a
        // wire the agent never got.
        let envelope_json = envelope
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("serialising message envelope")?;
        let legacy_author = match origin {
            "participant" => participant_slug.unwrap_or("user"),
            // 'system' and 'user' both land as 'user' for legacy readers.
            _ => "user",
        };
        // The participant is resolved INLINE by subquery rather than by a prior
        // awaited SELECT. `insert_message` delegates here and fires on every
        // text/tool_use/tool_result chunk, so a separate round trip per post is
        // a cost worth not paying — and one that would be near-impossible to
        // attribute once the send path starts routing through this method.
        //
        // The `CASE` is the origin guard the old two-step form carried in its
        // `match`: only a participant origin resolves, so a user/system row
        // stays NULL even if handed a live slug, and SQLite skips the subquery
        // entirely for those. A slug with no roster row resolves to NULL rather
        // than erroring, which is correct — `author` still carries the
        // attribution, and an unattributed row beats a lost one.
        let id = sqlx::query(
            "INSERT INTO messages \
             (session_id, participant_id, origin, kind, content, envelope, author, created_at) \
             VALUES (?1, \
                     CASE WHEN ?2 = 'participant' THEN \
                          (SELECT id FROM session_participants \
                           WHERE session_id = ?1 AND slug = ?3) END, \
                     ?2, ?4, ?5, ?6, ?7, ?8)",
        )
        // Bind order is NOT column order — `?3` appears only inside the CASE,
        // so position no longer tells you the target. The binds below are, in
        // order: ?1 session_id, ?2 origin, ?3 participant_slug, ?4 kind,
        // ?5 content, ?6 envelope, ?7 author, ?8 created_at.
        .bind(&*session_id)
        .bind(origin)
        .bind(participant_slug)
        .bind(kind)
        .bind(content.as_str())
        .bind(envelope_json.as_deref())
        .bind(legacy_author)
        // `now_utc()` (RFC3339-Z), NOT SQLite's `datetime('now')`, which this
        // method used until `insert_message` began delegating here. That is the
        // project baseline every other write already keeps — see `storage::time`
        // — and the difference is not cosmetic. `datetime('now')` emits a
        // zone-less `2026-08-06 11:22:33`, which sorts BEFORE the same instant
        // in RFC3339 (space < 'T'), so `has_message_from_author_since` — a
        // string compare against an RFC3339-Z bound — would have gone
        // permanently false and silently disarmed the findings re-raise
        // turn-evidence guard. The frontend would also have read every new row
        // as local time: the staleness hallucination `now_utc` exists to stop.
        .bind(now_utc())
        .execute(&self.pool)
        .await
        .with_context(|| format!("posting to channel for {session_id}"))?
        .last_insert_rowid();
        // The one and only place a `PersistedMessage` is minted, and it sits
        // downstream of the INSERT — the row is what the value proves. Every
        // field MOVES in; nothing is copied.
        Ok(PersistedMessage {
            message_id: id,
            session_id,
            body: content,
            envelope,
        })
    }

    /// The channel after `after_id`, oldest first — the read half, and the
    /// whole channel rather than any one participant's view of it.
    ///
    /// At most `limit` rows; [`ChannelPage::more`] says whether that cut the
    /// read short. The bound is a parameter rather than a constant because the
    /// two readers want different numbers: a turn's backlog is capped by
    /// [`UNREAD_BATCH_LIMIT`], while a whole-channel read is capped by whatever
    /// the reader can hold. What a participant WAKING on its turn reads is
    /// [`Storage::unread_for_participant`], not this — that one starts at the
    /// participant's cursor and leaves out its own rows.
    pub async fn channel_after(
        &self,
        session_id: &str,
        after_id: i64,
        limit: i64,
    ) -> Result<ChannelPage> {
        self.channel_page(session_id, after_id, None, limit).await
    }

    /// The one channel read. `exclude_participant` drops rows that participant
    /// AUTHORED, which is the difference between a whole-channel read and a
    /// backlog — see [`Storage::unread_for_participant`].
    async fn channel_page(
        &self,
        session_id: &str,
        after_id: i64,
        exclude_participant: Option<i64>,
        limit: i64,
    ) -> Result<ChannelPage> {
        // `?3 IS NULL OR …` rather than two query strings: one statement, one
        // plan, and the whole-channel read cannot drift from the backlog read.
        //
        // `participant_id IS NULL OR participant_id <> ?3` is the exclusion, and
        // the NULL half is load-bearing. `origin = 'user'` and `origin =
        // 'system'` rows carry no participant by design (0044), and SQL's
        // three-valued logic makes `NULL <> 5` NULL — never true — so without it
        // every user message and every host injection would vanish from every
        // backlog. An `origin = 'participant'` row that resolved to nobody is
        // kept for the same reason: it cannot be attributed to this participant,
        // so it cannot be its own.
        //
        // Reads `limit + 1` and keeps `limit`. Asking for one more row is how
        // "there is more" is learned from the same query rather than from a
        // second `count(*)` that could disagree with it under a concurrent
        // write.
        //
        // `saturating_add`, not `+`, because `i64::MAX` is a legal argument and
        // `i64::MAX + 1` is an overflow: a panic in debug, and in release a wrap
        // to a negative, which SQLite treats as NO LIMIT (verified:
        // `SELECT … LIMIT -1` returns every row) — silently restoring the
        // unbounded read this exists to remove. `.max(0)` for the other end: a
        // negative `limit` would otherwise reach SQLite unchanged and mean the
        // same thing. Clamped, it means what it says — zero rows, and `more`
        // true if any exist.
        let keep = limit.max(0);
        let probe = keep.saturating_add(1);
        let mut raw = sqlx::query(&format!(
            "SELECT {CHANNEL_COLUMNS} FROM messages \
             WHERE session_id = ?1 AND id > ?2 \
               AND (?3 IS NULL OR participant_id IS NULL OR participant_id <> ?3) \
             ORDER BY id ASC LIMIT ?4"
        ))
        .bind(session_id)
        .bind(after_id)
        .bind(exclude_participant)
        .bind(probe)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("reading channel for {session_id}"))?;
        // Truncate the ROWS before decoding them, not the decoded page after.
        // `channel_from_row` clones the body out of the row and the largest
        // single message in the live database is ~2 MB, so decoding the probe
        // row and dropping it is a whole-body copy for a value that exists only
        // to be counted.
        let more = raw.len() as i64 > keep;
        raw.truncate(keep as usize);
        Ok(ChannelPage {
            rows: raw.iter().map(channel_from_row).collect(),
            more,
        })
    }

    /// What this participant has not read yet. The query that makes "what did
    /// participant X actually receive?" answerable — a cursor range, not
    /// archaeology across a side table of drop records.
    ///
    /// **Excludes the participant's own rows.** It read them back before, which
    /// meant a participant handed its backlog met its own last turn as fresh
    /// input. Everything else past the cursor is delivered, including rows it
    /// was never "forwarded": the peer's turns, the user's messages and the
    /// host's `system` injections. Context completeness is structural.
    ///
    /// **Bounded at [`UNREAD_BATCH_LIMIT`] rows.** A participant that has never
    /// read used to get the entire session history in one `Vec` and then onto
    /// one wire; [`ChannelPage::more`] is how the caller learns to come back
    /// for the rest after committing this batch.
    pub async fn unread_for_participant(&self, participant_id: i64) -> Result<ChannelPage> {
        let Some(p) = self.participant_by_id(participant_id).await? else {
            return Ok(ChannelPage::default());
        };
        let cursor = self.cursor_for(participant_id).await?;
        self.channel_page(
            &p.session_id,
            cursor,
            Some(participant_id),
            UNREAD_BATCH_LIMIT,
        )
        .await
    }

    pub async fn participant_by_id(&self, id: i64) -> Result<Option<Participant>> {
        let row = sqlx::query(&format!(
            "SELECT {PARTICIPANT_COLUMNS} FROM session_participants WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context("loading participant by id")?;
        Ok(row.as_ref().map(participant_from_row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole channel, for the tests whose channels are far smaller than any
    /// bound. The `more` assertion is what keeps that true: a test that grows
    /// past the read limit must fail loudly rather than quietly assert against
    /// the first page.
    async fn all_rows(s: &Storage, session_id: &str) -> Vec<ChannelMessage> {
        let page = s.channel_after(session_id, 0, 100).await.unwrap();
        assert!(!page.more, "this test's channel outgrew its read limit");
        page.rows
    }

    /// 0044 is armed, so the stock in-memory backend has these tables — the
    /// transitional `storage_with_0044()` scaffold that applied the draft by
    /// hand is gone. Kept as a named alias so the tests still read as
    /// "storage that has 0044", which is the property they depend on.
    async fn storage_with_0044() -> Storage {
        Storage::memory().await.unwrap()
    }

    // ---- the wire renderer ----------------------------------------------

    #[test]
    fn no_envelope_renders_the_body_unchanged() {
        // The phase-change notice relies on this: `transition_notice()` already
        // carries its own `[PHASE: X]`, so an envelope would double-tag it.
        assert_eq!(render_wire(None, "advance to Apply"), "advance to Apply");
        assert_eq!(render_wire(Some(&Envelope::default()), "bare"), "bare");
    }

    /// Router inventory **#9**'s upgrade: the override tag was a sentence the
    /// router spliced onto the body, and is now a field. Same wording on the
    /// wire, so a session spanning task 14 sees no change.
    #[test]
    fn a_peer_ack_override_renders_its_tag_in_front_of_the_body() {
        let tagged = Envelope::default().with_peer_ack_override(true);
        assert_eq!(
            render_wire(Some(&tagged), "the verdict"),
            "[peer_ack overridden — this turn carried substantive text, so it was \
             forwarded anyway]\nthe verdict"
        );

        // `false` renders nothing — the tag must not ride an ordinary turn.
        let plain = Envelope::default().with_peer_ack_override(false);
        assert_eq!(render_wire(Some(&plain), "the verdict"), "the verdict");

        // Order: after the system prefix, closest to the body — the shape the
        // router built by hand, where the tag was concatenated onto the body and
        // everything else wrapped the pair.
        let both = Envelope::default()
            .with_system_prefix("HOST NOTE")
            .with_peer_ack_override(true);
        assert_eq!(
            render_wire(Some(&both), "the verdict"),
            "HOST NOTE\n[peer_ack overridden — this turn carried substantive text, so it \
             was forwarded anyway]\nthe verdict"
        );
    }

    #[test]
    fn the_phase_tag_leads_and_the_body_trails() {
        assert_eq!(
            render_wire(Some(&Envelope::phase("Apply")), "go"),
            "[PHASE: Apply]\ngo"
        );
    }

    #[test]
    fn the_findings_banner_sits_between_the_phase_tag_and_the_body() {
        // Zero open findings is the common case and must cost nothing — the
        // banner is salience, and an empty one would train agents to skim it.
        assert_eq!(
            render_wire(Some(&Envelope::phase("Verify").with_open_blocking(0)), "go"),
            render_wire(Some(&Envelope::phase("Verify")), "go")
        );
        let wire = render_wire(Some(&Envelope::phase("Verify").with_open_blocking(3)), "go");
        assert_eq!(
            wire,
            "[PHASE: Verify]\n⚠ 3 unresolved EYES blocking finding(s) — run \
             check_open_findings and disposition each (fix/rebut) before you \
             commit.\ngo"
        );
    }

    #[test]
    fn the_system_prefix_sits_closest_to_the_body() {
        // The only site with a prefix is the user broadcast (the post-cancel
        // reconcile directive), and it built `{prefix}\n{body}` FIRST, then
        // wrapped that whole thing in the phase envelope. Hence innermost.
        let envelope = Envelope::phase("Apply")
            .with_open_blocking(1)
            .with_system_prefix("[System: previous turn interrupted]");
        let wire = render_wire(Some(&envelope), "do the thing");
        assert!(wire.starts_with("[PHASE: Apply]\n⚠ 1 unresolved"));
        assert!(wire.ends_with("[System: previous turn interrupted]\ndo the thing"));
    }

    #[test]
    fn an_envelope_survives_a_round_trip_through_the_column() {
        // The column holds JSON, so the renderer's input on the read side is
        // whatever `serde` gives back. A field that serialises but does not
        // deserialise would render a SHORTER wire than the one delivered, and
        // the chat pane would quietly disagree with what the agent read.
        let envelope = Envelope::phase("Plan")
            .with_open_blocking(2)
            .with_system_prefix("[System: note]");
        let json = serde_json::to_string(&envelope).unwrap();
        let back: Envelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, envelope);
        assert_eq!(render_wire(Some(&back), "b"), render_wire(Some(&envelope), "b"));
        // Absent fields are omitted rather than written as nulls/zeroes: the
        // common case is phase-only and it rides on ~200k existing rows'
        // worth of table, so the column value stays minimal.
        assert_eq!(
            serde_json::to_string(&Envelope::phase("Plan")).unwrap(),
            r#"{"phase":"Plan"}"#
        );
    }

    #[tokio::test]
    async fn the_draft_seeds_the_users_two_roles() {
        let s = storage_with_0044().await;
        let roles = s.list_roles().await.unwrap();
        assert_eq!(roles.len(), 2, "hands + eyes");
        let hands = s.role_by_slug("hands").await.unwrap().expect("hands");
        assert!(hands.builtin);
        assert_eq!(hands.participation_mode, "active");
        assert!(hands.capabilities.contains("edit_files"));
        let eyes = s.role_by_slug("eyes").await.unwrap().expect("eyes");
        assert!(eyes.capabilities.contains("file_finding"));
        assert!(
            !eyes.capabilities.contains("edit_files"),
            "EYES must not be seeded with write access"
        );
    }

    // ---- roles CRUD (B8a) -------------------------------------------------

    /// A minimal legal draft. Tests override only the field under test, so a
    /// new required field breaks compilation here rather than silently
    /// defaulting in twenty places.
    fn draft(display_name: &str) -> RoleDraft {
        RoleDraft {
            display_name: display_name.to_string(),
            slug: None,
            description_prompt: None,
            capabilities: r#"["read_channel","post_channel"]"#.to_string(),
            participation_mode: "active".to_string(),
            default_model_id: None,
        }
    }

    #[test]
    fn slugify_keeps_ascii_alphanumerics_and_collapses_everything_else() {
        assert_eq!(slugify("HANDS"), "hands");
        assert_eq!(slugify("Code Reviewer"), "code-reviewer");
        // Runs collapse to ONE hyphen, and both ends are trimmed — otherwise a
        // name pasted with trailing spaces yields a slug ending in `-`, which
        // reads as a truncated key.
        assert_eq!(slugify("  Deep  //  Thinker!!  "), "deep-thinker");
        assert_eq!(slugify("v2"), "v2");
        // The documented cost of ASCII-only, asserted rather than described:
        // an accent is dropped, and a wholly non-Latin name has no slug at all
        // and falls back rather than inserting an empty UNIQUE key.
        assert_eq!(slugify("Café"), "caf");
        assert_eq!(slugify("日本語"), FALLBACK_SLUG);
        assert_eq!(slugify(""), FALLBACK_SLUG);
        assert_eq!(slugify("!!!"), FALLBACK_SLUG);
    }

    #[test]
    fn a_colliding_slug_takes_the_first_free_numeric_suffix() {
        let taken: HashSet<String> = ["hands", "hands-2", "hands-4"]
            .into_iter()
            .map(String::from)
            .collect();
        // Free base wins outright — the common case must not be suffixed.
        assert_eq!(first_free_slug("eyes", &taken), "eyes");
        // Suffixes start at 2 (the base is the "1") and skip what is held, so
        // the run is `hands`, `hands-2`, `hands-3` — NOT `hands-5`, which is
        // what "one past the highest" would produce.
        assert_eq!(first_free_slug("hands", &taken), "hands-3");
        // Densely packed: every candidate up to the bound is taken except the
        // last. This is the case that proves the `expect` is unreachable
        // rather than lucky — N taken slugs cannot exhaust N+2 candidates.
        let dense: HashSet<String> = std::iter::once("r".to_string())
            .chain((2..=4).map(|n| format!("r-{n}")))
            .collect();
        assert_eq!(first_free_slug("r", &dense), "r-5");
    }

    #[test]
    fn capabilities_must_be_a_json_array_of_strings() {
        // Normalised: whitespace goes, so the column holds one spelling.
        assert_eq!(
            canonical_capabilities(r#"[ "read_channel" ,  "post_channel" ]"#).unwrap(),
            r#"["read_channel","post_channel"]"#
        );
        assert_eq!(canonical_capabilities("[]").unwrap(), "[]");
        // Order and duplicates survive: silently reordering or de-duplicating
        // would be an edit of what the caller asked to store.
        assert_eq!(
            canonical_capabilities(r#"["b","a","b"]"#).unwrap(),
            r#"["b","a","b"]"#
        );
        // The rejections. Each of these decodes to "no capabilities" through
        // `CapabilitySet::from_slugs`, which is a LEGAL configuration (an
        // observer) — so accepting them would make a malformed write
        // indistinguishable from a deliberate one.
        for bad in ["null", "{}", r#""read_channel""#, "[1,2]", r#"["ok",3]"#, "nonsense"] {
            assert!(
                canonical_capabilities(bad).is_err(),
                "{bad} must not be storable as a capability set"
            );
        }
    }

    #[tokio::test]
    async fn creating_a_role_derives_its_slug_and_returns_what_was_stored() {
        let s = storage_with_0044().await;
        // Padded on purpose: a name arrives from a text field and carries
        // whatever the user pasted. It is stored TRIMMED, so the list does not
        // render a row that looks indented, and so the blank-name guard's
        // `trim().is_empty()` agrees with what actually gets written.
        let mut d = draft("  Code Reviewer  ");
        d.description_prompt = Some("be terse".into());
        d.default_model_id = Some("m1".into());
        d.capabilities = r#"[ "read_channel" , "file_finding" ]"#.into();
        d.participation_mode = "observer".into();
        let created = s.create_role(&d).await.unwrap();

        assert_eq!(created.slug, "code-reviewer");
        assert_eq!(created.display_name, "Code Reviewer");
        assert_eq!(created.description_prompt.as_deref(), Some("be terse"));
        // D8: the Roles tab owns the default model, so this column has to
        // round-trip or the tab's model select is a control that does nothing.
        assert_eq!(created.default_model_id.as_deref(), Some("m1"));
        assert_eq!(created.participation_mode, "observer");
        assert_eq!(created.capabilities, r#"["read_channel","file_finding"]"#);
        // A user-created role is not a bot-hq seed, whatever it is called.
        assert!(!created.builtin);
        assert!(!created.archived);
        // The returned value is the stored row, not the draft echoed back.
        assert_eq!(s.role_by_id(created.id).await.unwrap().as_ref(), Some(&created));
        assert!(s.list_roles().await.unwrap().contains(&created));
    }

    #[tokio::test]
    async fn a_created_role_never_steals_an_existing_slug() {
        let s = storage_with_0044().await;
        // "HANDS" slugifies onto the seeded role's slug.
        let first = s.create_role(&draft("HANDS")).await.unwrap();
        assert_eq!(first.slug, "hands-2");
        let second = s.create_role(&draft("hands")).await.unwrap();
        assert_eq!(second.slug, "hands-3");
        // The seeded role is untouched — a collision must not overwrite.
        let seeded = s.role_by_slug("hands").await.unwrap().unwrap();
        assert!(seeded.builtin);
        assert!(seeded.capabilities.contains("edit_files"));
    }

    #[tokio::test]
    async fn a_caller_supplied_slug_is_normalised_not_taken_verbatim() {
        let s = storage_with_0044().await;
        let mut d = draft("Anything At All");
        d.slug = Some("My Custom Slug!".into());
        let created = s.create_role(&d).await.unwrap();
        // The slug is a typed key and seeds the participant handle the rc3
        // mention syntax parses; a space in it would not survive that.
        assert_eq!(created.slug, "my-custom-slug");
    }

    #[tokio::test]
    async fn an_archived_slug_stays_reserved() {
        let s = storage_with_0044().await;
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        s.set_role_archived(hands.id, true).await.unwrap();
        // `roles.slug` is UNIQUE over the WHOLE table (0047 did not scope it to
        // live rows), so a generator that only counted live roles would hand
        // out `hands` here and then fail at the INSERT.
        let created = s.create_role(&draft("HANDS")).await.unwrap();
        assert_eq!(created.slug, "hands-2");
        // …and the archived original is still reachable, so it can come back.
        assert_eq!(
            s.role_by_slug("hands").await.unwrap().map(|r| r.id),
            Some(hands.id)
        );
    }

    #[tokio::test]
    async fn a_role_write_refuses_an_unknown_mode_or_a_malformed_capability_set() {
        let s = storage_with_0044().await;
        let mut bad_mode = draft("Watcher");
        bad_mode.participation_mode = "Active".into();
        // Capitalised, and `next_active_participant` filters the ring on the
        // exact string "active" — so this role's participants would be enabled,
        // listed, and never handed a turn.
        assert!(s.create_role(&bad_mode).await.is_err());

        let mut bad_caps = draft("Watcher");
        bad_caps.capabilities = "{}".into();
        assert!(s.create_role(&bad_caps).await.is_err());

        let mut blank = draft("   ");
        blank.slug = Some("watcher".into());
        assert!(s.create_role(&blank).await.is_err(), "a role needs a name");

        // Nothing partial was written by any of the three.
        assert_eq!(s.list_roles_including_archived().await.unwrap().len(), 2);

        // The same checks guard UPDATE, not just INSERT — a role edited into an
        // unschedulable mode is the identical failure, arrived at later.
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        assert!(s.update_role(hands.id, &bad_mode).await.is_err());
        assert!(s.update_role(hands.id, &bad_caps).await.is_err());
        assert_eq!(
            s.role_by_id(hands.id).await.unwrap().unwrap(),
            hands,
            "a refused update must change nothing"
        );
    }

    #[tokio::test]
    async fn renaming_a_role_leaves_its_slug_alone_so_roster_seeding_still_resolves() {
        let s = storage_with_0044().await;
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        // Padded, for the same reason `create_role`'s test is: an edit stores
        // the trimmed name, not the field's raw contents.
        let mut d = draft("  Executor  ");
        d.capabilities = hands.capabilities.clone();
        d.description_prompt = Some("drive the work".into());
        let renamed = s.update_role(hands.id, &d).await.unwrap();
        assert_eq!(renamed.display_name, "Executor");
        assert_eq!(renamed.slug, "hands", "a rename must not re-derive the slug");
        // The design's "ONLY stored prose" — the role's identity layer. An
        // update that dropped it would blank what the user just typed, and the
        // tab would redraw the empty box as if the save had taken.
        assert_eq!(renamed.description_prompt.as_deref(), Some("drive the work"));

        // The reason it must not. `ensure_session_roster` seeds every new
        // session from two literal `WHERE slug = 'hands' / 'eyes'` subqueries;
        // against a re-derived slug those resolve to NULL, and the session gets
        // a roster whose `role_id` is NULL with nothing reporting an error.
        s.create_session("s1", "t", None).await.unwrap();
        s.ensure_session_roster("s1").await.unwrap();
        let roster = s.participants_for_session("s1").await.unwrap();
        assert_eq!(roster.len(), 2);
        assert_eq!(roster[0].role_id, Some(hands.id));
        assert!(roster.iter().all(|p| p.role_id.is_some()));
    }

    #[tokio::test]
    async fn an_explicit_rename_moves_the_slug_and_is_idempotent() {
        let s = storage_with_0044().await;
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        let mut d = draft("Executor");
        d.slug = Some("Executor".into());
        assert_eq!(s.update_role(hands.id, &d).await.unwrap().slug, "executor");

        // Saving the same form again must NOT find `executor` taken by the row
        // being saved and slide it to `executor-2` — which is what saving twice
        // would do without `taken_slugs`' self-exclusion.
        assert_eq!(s.update_role(hands.id, &d).await.unwrap().slug, "executor");

        // A DIFFERENT role asking for the same slug still gets suffixed.
        let other = s.create_role(&draft("Eyes")).await.unwrap();
        let mut clash = draft("Executor");
        clash.slug = Some("executor".into());
        assert_eq!(s.update_role(other.id, &clash).await.unwrap().slug, "executor-2");
    }

    #[tokio::test]
    async fn updating_a_role_leaves_provenance_archival_state_and_live_snapshots_alone() {
        let s = storage_with_0044().await;
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        s.create_session("s1", "t", None).await.unwrap();
        let pid = s
            .insert_participant("s1", "brian", "Brian", Some(hands.id), None,
                                &hands.capabilities, "active", 0)
            .await
            .unwrap();
        s.set_role_archived(hands.id, true).await.unwrap();

        let mut d = draft("Executor");
        d.capabilities = r#"[ "read_channel" ]"#.into();
        d.participation_mode = "observer".into();
        let updated = s.update_role(hands.id, &d).await.unwrap();

        // The edit lands, normalised the same way a create is — an update that
        // stored the field's raw text would leave the column holding a second
        // spelling of the same set.
        assert_eq!(updated.capabilities, r#"["read_channel"]"#);
        // And the mode is the caller's, not a default. Pinned here because
        // demoting a role to `observer` is how the ring stops scheduling it:
        // an update that ignored this would leave the role looking demoted in
        // the tab while its participants kept taking turns.
        assert_eq!(updated.participation_mode, "observer");

        // `builtin` records that bot-hq seeded this row; editing it does not
        // un-seed it (0044: seeds are user-editable, and the flag is what lets
        // the UI offer "restore defaults").
        assert!(updated.builtin);
        // `archived` moves only through `set_role_archived`, so a save from a
        // form that never rendered the flag cannot resurrect a removed role.
        assert!(updated.archived);
        // The invite-time snapshot is the whole point of duplicating
        // capabilities onto the participant: narrowing the role must not
        // narrow — or widen — a participant that is already running.
        let live = s.participant_by_id(pid).await.unwrap().unwrap();
        assert_eq!(live.capabilities, hands.capabilities);
        assert!(live.capabilities.contains("edit_files"));
        assert_eq!(live.role_id, Some(hands.id));
    }

    #[tokio::test]
    async fn archiving_hides_a_role_from_the_picker_without_destroying_it() {
        let s = storage_with_0044().await;
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        s.set_role_archived(hands.id, true).await.unwrap();

        let live = s.list_roles().await.unwrap();
        assert_eq!(live.len(), 1, "archived roles leave the picker");
        assert_eq!(live[0].slug, "eyes");
        // Still a row: the id a past session's `role_id` points at resolves,
        // and the tab can offer an un-archive.
        let all = s.list_roles_including_archived().await.unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|r| r.id == hands.id && r.archived));
        assert!(s.role_by_id(hands.id).await.unwrap().unwrap().archived);
        assert!(s.role_by_slug("hands").await.unwrap().unwrap().archived);

        s.set_role_archived(hands.id, false).await.unwrap();
        assert_eq!(s.list_roles().await.unwrap().len(), 2);
        assert!(!s.role_by_id(hands.id).await.unwrap().unwrap().archived);
    }

    #[tokio::test]
    async fn writing_to_a_role_that_does_not_exist_is_an_error_not_a_no_op() {
        let s = storage_with_0044().await;
        // An UPDATE that matches nothing is not an error in SQLite, so without
        // the row-count check both of these would report success and change
        // nothing — the quietest possible failure.
        assert!(s.update_role(9999, &draft("Ghost")).await.is_err());
        assert!(s.set_role_archived(9999, true).await.is_err());
        // And a read for an id nothing holds is `None`, not "whatever row sorts
        // first". `AUTOINCREMENT` never assigns 0, so this is the id every
        // uninitialised caller passes — and a lookup loosened from `id = ?` to
        // a range would answer it with the `hands` role.
        assert!(s.role_by_id(0).await.unwrap().is_none());
        assert!(s.role_by_id(9999).await.unwrap().is_none());
        // Same for the slug lookup, and the name is chosen to sort BEFORE both
        // seeded slugs: a lookup loosened to a range would answer this with
        // `eyes` rather than nothing, and `create_role` reads the taken set
        // through the same table.
        assert!(s.role_by_slug("absent-role").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_role_a_participant_references_cannot_be_hard_deleted() {
        // The premise of migration 0047, checked against the real schema rather
        // than asserted in its comment. `session_participants.role_id` REFERENCES
        // `roles(id)` with no ON DELETE clause, and `Storage::memory` connects
        // with foreign_keys ON, so the delete is REFUSED — which is why removal
        // had to become archival.
        let s = storage_with_0044().await;
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        s.create_session("s1", "t", None).await.unwrap();
        s.insert_participant("s1", "brian", "Brian", Some(hands.id), None,
                             &hands.capabilities, "active", 0)
            .await
            .unwrap();

        let err = sqlx::query("DELETE FROM roles WHERE id = ?")
            .bind(hands.id)
            .execute(s.pool())
            .await
            .expect_err("the FK must refuse this");
        assert!(
            err.to_string().to_uppercase().contains("FOREIGN KEY"),
            "expected an FK refusal, got {err}"
        );
        assert!(s.role_by_id(hands.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn inviting_a_participant_seeds_its_cursor() {
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        let pid = s
            .insert_participant("s1", "brian", "Brian", Some(hands.id), None,
                                &hands.capabilities, "active", 0)
            .await
            .unwrap();
        // Every participant reads the channel, so a cursor exists from birth —
        // a participant without one would be invisibly undeliverable.
        assert_eq!(s.cursor_for(pid).await.unwrap(), 0);
        let roster = s.participants_for_session("s1").await.unwrap();
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].turn_position, 0);
        assert!(!roster[0].done_vote);
    }

    // `cursors_only_move_forward` lived here. Its invariant did not move — it
    // is pinned by `a_committed_delivery_never_rewinds_a_cursor`, through
    // `commit_delivery`, which is now the only way the cursor moves.

    /// A roster row with nothing behind it but its ring-relevant fields.
    /// [`next_in_ring`] reads `id`, `turn_position` and `slug`, so the rest is
    /// filler — and the point of the pure function is that these rosters need
    /// not be constructible in the database.
    fn ring_member(id: i64, slug: &str, turn_position: i64) -> Participant {
        Participant {
            id,
            session_id: "s1".into(),
            slug: slug.into(),
            display_name: slug.into(),
            role_id: None,
            model_id: None,
            runtime: "claude_code".into(),
            capabilities: "[]".into(),
            participation_mode: "active".into(),
            turn_position,
            done_vote: false,
            enabled: true,
        }
    }

    /// Walk the ring `steps` times from a cold start, returning the slugs in
    /// the order they were handed the turn.
    fn walk(ring: &[&Participant], steps: usize) -> Vec<String> {
        let mut current: Option<&Participant> = None;
        let mut seen = Vec::new();
        for _ in 0..steps {
            let next = next_in_ring(ring, current).expect("a non-empty ring hands out a turn");
            seen.push(next.slug.clone());
            current = Some(next);
        }
        seen
    }

    #[test]
    fn every_active_participant_gets_a_turn_even_at_a_shared_position() {
        // The starvation defect. 0044 indexes `turn_position` NON-uniquely and
        // DEFAULTs it to 0, so A(0), B(0), C(1) is a roster the schema permits.
        // Advancing on `turn_position > current` then runs a, c, a, c: B is
        // never scheduled, and because it is still enabled and active,
        // consensus keeps waiting on a vote it can never be given the turn to
        // cast. Migration 0045 makes this roster unrepresentable; the ring must
        // not depend on that to schedule everyone.
        let a = ring_member(1, "a", 0);
        let b = ring_member(2, "b", 0);
        let c = ring_member(3, "c", 1);
        assert_eq!(
            walk(&[&a, &b, &c], 6),
            ["a", "b", "c", "a", "b", "c"],
            "a shared position must cost an ordering, not a participant"
        );
    }

    #[test]
    fn a_participant_that_left_the_rotation_mid_turn_still_advances() {
        // The sequencer hands the turn to X and X is disabled (or demoted to
        // observer) before it completes. `current` is then not IN the ring, so
        // there is no "one place along" to step — the ring has to fall back to
        // the first member that sorts after it, and wrap when there is none.
        let a = ring_member(1, "a", 0);
        let gone = ring_member(2, "gone", 1);
        let c = ring_member(3, "c", 2);
        let ring = [&a, &c];
        assert_eq!(
            next_in_ring(&ring, Some(&gone)).unwrap().slug,
            "c",
            "advance past where the departed participant sat"
        );
        let last = ring_member(9, "last", 7);
        assert_eq!(
            next_in_ring(&ring, Some(&last)).unwrap().slug,
            "a",
            "and wrap when nothing sorts after it"
        );
    }

    #[test]
    fn an_empty_ring_hands_out_no_turn() {
        let a = ring_member(1, "a", 0);
        assert!(next_in_ring(&[], None).is_none());
        assert!(next_in_ring(&[], Some(&a)).is_none());
    }

    #[tokio::test]
    async fn the_ring_skips_observers_and_wraps() {
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.insert_participant("s1", "a", "A", None, None, "[]", "active", 0).await.unwrap();
        s.insert_participant("s1", "obs", "Obs", None, None, "[]", "observer", 1).await.unwrap();
        s.insert_participant("s1", "c", "C", None, None, "[]", "active", 2).await.unwrap();

        // A user message resets to the first active participant.
        let first = s.next_active_participant("s1", None).await.unwrap().unwrap();
        assert_eq!(first.slug, "a");
        // The observer is SKIPPED, not given a no-op turn.
        let second = s.next_active_participant("s1", Some(&first)).await.unwrap().unwrap();
        assert_eq!(second.slug, "c", "observer must not take a turn");
        // The ring wraps.
        let third = s.next_active_participant("s1", Some(&second)).await.unwrap().unwrap();
        assert_eq!(third.slug, "a");
    }

    #[tokio::test]
    async fn two_active_participants_cannot_share_a_turn_slot() {
        // The schema half of the starvation defect. The roster
        // `every_active_participant_gets_a_turn_even_at_a_shared_position`
        // walks is one 0044 represented happily — non-unique index, DEFAULT 0,
        // and `insert_participant` taking the position unchecked. Migration
        // 0045 is what stops the database holding it.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.insert_participant("s1", "a", "A", None, None, "[]", "active", 0).await.unwrap();

        let err = s
            .insert_participant("s1", "b", "B", None, None, "[]", "active", 0)
            .await
            .expect_err("two actives sharing a slot must be rejected");
        assert!(
            format!("{err:#}").contains("UNIQUE constraint failed"),
            "expected a uniqueness failure, got: {err:#}"
        );

        // What it does NOT prevent, and must not: an observer at the same
        // position, because an observer never takes a turn and its position is
        // retained purely against a later promotion.
        s.insert_participant("s1", "obs", "Obs", None, None, "[]", "observer", 0)
            .await
            .unwrap();
        // …nor slot 0 of a different session. The slot is per session.
        s.create_session("s2", "t", None).await.unwrap();
        s.insert_participant("s2", "a", "A", None, None, "[]", "active", 0).await.unwrap();
    }

    #[tokio::test]
    async fn the_slot_constraint_covers_every_row_the_ring_schedules() {
        // 0045's predicate and the ring's filter have to select the SAME rows,
        // and they did not at first. `enabled` is `INTEGER NOT NULL DEFAULT 1`
        // with no CHECK — a truthiness flag, not a two-valued one — while
        // `participant_from_row` decodes it as `!= 0`. Written `WHERE enabled =
        // 1`, the index skipped a row storing 2 that the ring still scheduled,
        // which is the starvation roster back through a hole in the predicate.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let a = s
            .insert_participant("s1", "a", "A", None, None, "[]", "active", 0)
            .await
            .unwrap();

        // First: such a row IS in the rotation, so the index must cover it.
        // Parked at a free slot so this half tests scheduling, not uniqueness.
        sqlx::query(
            "INSERT INTO session_participants \
             (session_id, slug, display_name, capabilities, participation_mode, \
              turn_position, enabled) \
             VALUES ('s1', 'truthy', 'Truthy', '[]', 'active', 5, 2)",
        )
        .execute(s.pool())
        .await
        .unwrap();
        let truthy = s.participant_by_slug("s1", "truthy").await.unwrap().unwrap();
        assert!(truthy.enabled, "`enabled = 2` decodes as enabled");
        let first = s.next_active_participant("s1", None).await.unwrap().unwrap();
        assert_eq!(first.id, a);
        assert_eq!(
            s.next_active_participant("s1", Some(&first)).await.unwrap().unwrap().id,
            truthy.id,
            "the ring schedules it, so the slot constraint has to apply to it"
        );

        // Second: and it therefore collides like any other rotation member.
        let err = sqlx::query(
            "INSERT INTO session_participants \
             (session_id, slug, display_name, capabilities, participation_mode, \
              turn_position, enabled) \
             VALUES ('s1', 'b', 'B', '[]', 'active', 5, 2)",
        )
        .execute(s.pool())
        .await
        .expect_err("a row the ring schedules must not share a slot");
        assert!(
            err.to_string().contains("UNIQUE constraint failed"),
            "expected a uniqueness failure, got: {err}"
        );
    }

    #[tokio::test]
    async fn a_slot_is_occupied_only_while_someone_is_in_the_rotation() {
        // Why the index is PARTIAL rather than over every row. A disabled
        // participant keeps its `turn_position`, so a full unique index would
        // reserve the slot for a row that takes no turns — and re-inviting at
        // that position would collide with nobody.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let a = s
            .insert_participant("s1", "a", "A", None, None, "[]", "active", 0)
            .await
            .unwrap();
        sqlx::query("UPDATE session_participants SET enabled = 0 WHERE id = ?")
            .bind(a)
            .execute(s.pool())
            .await
            .unwrap();

        let b = s
            .insert_participant("s1", "b", "B", None, None, "[]", "active", 0)
            .await
            .expect("a vacated slot is free to re-invite into");

        // And the constraint follows the rotation rather than the INSERT:
        // re-enabling A now moves a row INTO the set, which is the moment the
        // collision starts mattering, so that is where it fails.
        let err = sqlx::query("UPDATE session_participants SET enabled = 1 WHERE id = ?")
            .bind(a)
            .execute(s.pool())
            .await
            .expect_err("re-enabling onto an occupied slot must be rejected");
        assert!(
            err.to_string().contains("UNIQUE constraint failed"),
            "expected a uniqueness failure, got: {err}"
        );
        assert_eq!(
            s.next_active_participant("s1", None).await.unwrap().unwrap().id,
            b,
            "and the rotation is still B's alone"
        );
    }

    #[tokio::test]
    async fn consensus_needs_every_active_participant_and_ignores_observers() {
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let a = s.insert_participant("s1", "a", "A", None, None, "[]", "active", 0).await.unwrap();
        let c = s.insert_participant("s1", "c", "C", None, None, "[]", "active", 1).await.unwrap();
        s.insert_participant("s1", "obs", "O", None, None, "[]", "observer", 2).await.unwrap();

        assert!(!s.all_active_voted_done("s1").await.unwrap());
        s.set_done_vote(a, true).await.unwrap();
        assert!(!s.all_active_voted_done("s1").await.unwrap(), "one vote is not consensus");
        s.set_done_vote(c, true).await.unwrap();
        assert!(
            s.all_active_voted_done("s1").await.unwrap(),
            "observers must not be required to vote — 1 active + 3 observers \
             would otherwise need 4 yields to halt"
        );
        // Any substantive output resets every vote.
        s.clear_done_votes("s1").await.unwrap();
        assert!(!s.all_active_voted_done("s1").await.unwrap());
    }

    #[tokio::test]
    async fn a_withheld_delivery_is_still_a_visible_row() {
        // The redesign's core inversion: today a suppressed forward vanishes.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let pid = s
            .insert_participant("s1", "a", "A", None, None, "[]", "active", 0)
            .await
            .unwrap();
        s.commit_delivery(pid, &[(1, None), (2, Some("spin"))]).await.unwrap();
        let withheld = s.withheld_for_participant(pid).await.unwrap();
        assert_eq!(withheld, vec![(2, "spin".to_string())]);
    }

    /// Every delivery row for a participant, as `(message_id, withheld_reason)`
    /// — including the delivered ones, which `withheld_for_participant`
    /// deliberately does not return.
    async fn delivery_rows(s: &Storage, participant_id: i64) -> Vec<(i64, Option<String>)> {
        sqlx::query_as(
            "SELECT message_id, withheld_reason FROM participant_deliveries \
             WHERE participant_id = ? ORDER BY message_id",
        )
        .bind(participant_id)
        .fetch_all(s.pool())
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_batch_of_deliveries_and_its_cursor_land_in_one_commit() {
        // `record_delivery` and `advance_cursor` were two public methods with
        // no transaction between them, so a caller pairing them by hand — the
        // only way to deliver anything — left a window where a crash advanced
        // the cursor with no delivery rows behind it. The module's claim to
        // answer "what did participant X receive?" is exactly what that window
        // costs. There is now one call and one COMMIT.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.ensure_session_roster("s1").await.unwrap();
        let brian = s.participant_by_slug("s1", "brian").await.unwrap().unwrap().id;
        let rain = s.participant_by_slug("s1", "rain").await.unwrap().unwrap().id;

        let m1 = s.post_to_channel("s1", "user", None, "text", "one", None).await.unwrap();
        let m2 = s
            .post_to_channel("s1", "participant", Some("brian"), "text", "two", None)
            .await
            .unwrap();
        let m3 = s.post_to_channel("s1", "user", None, "text", "three", None).await.unwrap();

        let backlog = s.unread_for_participant(rain).await.unwrap().rows;
        assert_eq!(backlog.len(), 3, "precondition: three rows past the cursor");
        s.commit_delivery(
            rain,
            &[
                (m1.message_id(), None),
                (m2.message_id(), Some("spin")),
                (m3.message_id(), None),
            ],
        )
        .await
        .unwrap();

        // Every row in the batch has a record, withheld or not…
        assert_eq!(
            delivery_rows(&s, rain).await,
            vec![
                (m1.message_id(), None),
                (m2.message_id(), Some("spin".to_string())),
                (m3.message_id(), None),
            ]
        );
        // …and the cursor sat past the whole batch in the same commit.
        assert_eq!(s.cursor_for(rain).await.unwrap(), m3.message_id());
        assert!(
            s.unread_for_participant(rain).await.unwrap().rows.is_empty(),
            "a withheld row is recorded, not re-offered forever"
        );
        // Scoped to one participant: Brian's cursor and records are untouched.
        assert_eq!(s.cursor_for(brian).await.unwrap(), 0);
        assert!(delivery_rows(&s, brian).await.is_empty());
    }

    #[tokio::test]
    async fn a_delivery_with_no_cursor_to_move_is_an_error() {
        // The cursor half is `UPDATE … WHERE participant_id = ?`, and in SQLite
        // an UPDATE that matches nothing is not an error — zero rows changed,
        // `Ok(())` back. A participant whose cursor row is missing would get its
        // deliveries recorded, its cursor left at 0, and the same batch
        // re-offered every turn forever, while every call reported success. The
        // transaction does not help: there is nothing to roll back.
        //
        // Not reachable today — `insert_participant` and `ensure_session_roster`
        // both seed a cursor. But `ensure_session_roster` returns early when it
        // inserts nothing, BEFORE its cursor-seeding statement, so nothing in
        // the system would ever heal a participant that lost one.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let pid = s
            .insert_participant("s1", "a", "A", None, None, "[]", "active", 0)
            .await
            .unwrap();
        sqlx::query("DELETE FROM participant_cursors WHERE participant_id = ?")
            .bind(pid)
            .execute(s.pool())
            .await
            .unwrap();

        let err = s
            .commit_delivery(pid, &[(1, None)])
            .await
            .expect_err("a delivery whose cursor cannot move must not report success");
        assert!(
            format!("{err:#}").contains("no cursor"),
            "the error must name the missing cursor, got: {err:#}"
        );
        // And the whole batch rolled back: no half-recorded delivery either.
        assert!(delivery_rows(&s, pid).await.is_empty());
    }

    #[tokio::test]
    async fn a_committed_delivery_never_rewinds_a_cursor() {
        // Cursors only move FORWARD — a rewind re-delivers messages an agent has
        // already acted on. Carried over from `advance_cursor`, which this
        // replaces as the only way the cursor moves.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let pid = s
            .insert_participant("s1", "a", "A", None, None, "[]", "active", 0)
            .await
            .unwrap();
        s.commit_delivery(pid, &[(10, None)]).await.unwrap();
        assert_eq!(s.cursor_for(pid).await.unwrap(), 10);
        s.commit_delivery(pid, &[(4, None)]).await.unwrap();
        assert_eq!(s.cursor_for(pid).await.unwrap(), 10, "cursor must not rewind");
        // The late row is still RECORDED as delivered, though — the cursor is
        // where reading got to, not the list of what was handed over.
        assert_eq!(delivery_rows(&s, pid).await, vec![(4, None), (10, None)]);
        // An empty batch is a no-op rather than a cursor reset.
        s.commit_delivery(pid, &[]).await.unwrap();
        assert_eq!(s.cursor_for(pid).await.unwrap(), 10);
        assert_eq!(delivery_rows(&s, pid).await.len(), 2);
    }

    #[tokio::test]
    async fn every_legacy_message_query_still_works_after_0044() {
        // **This is the app-boot check, in miniature.** The runbook's one
        // outstanding unknown was whether the app can boot against the new
        // schema; SQL guards cannot answer that, because the failure mode is a
        // query that no longer matches the table.
        //
        // It caught a real defect: `origin` was NOT NULL with no default, so
        // every legacy `insert_message` — which knows nothing about `origin` —
        // would have failed on insert and the app would not have started. The
        // column is transitional-nullable because of this test.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();

        // The write path, untouched by B3a.
        let id = s
            .insert_message("s1", Author::Brian, MessageKind::Text, "hello")
            .await
            .unwrap()
            .message_id();
        assert!(id > 0, "legacy insert_message must still work post-0044");
        s.insert_message("s1", Author::User, MessageKind::Text, "hi back")
            .await
            .unwrap();

        // The read paths that key on `author`.
        assert_eq!(
            s.count_user_messages("s1").await.unwrap(),
            1,
            "count_user_messages keys on author = 'user'"
        );
        let msgs = s.messages_for_session("s1", None).await.unwrap();
        assert_eq!(msgs.len(), 2, "both rows readable through the legacy path");
        assert_eq!(msgs[0].author, Author::Brian.as_str());
    }

    #[tokio::test]
    async fn the_channel_records_what_a_participant_receives() {
        // The redesign's central inversion. Today a peer forward is built,
        // pushed to stdin, and never persisted — so "what did Rain actually
        // read?" is unanswerable. Here it is a cursor range.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        let eyes = s.role_by_slug("eyes").await.unwrap().unwrap();
        let b = s
            .insert_participant("s1", "brian", "Brian", Some(hands.id), None,
                                &hands.capabilities, "active", 0)
            .await
            .unwrap();
        let r = s
            .insert_participant("s1", "rain", "Rain", Some(eyes.id), None,
                                &eyes.capabilities, "active", 1)
            .await
            .unwrap();

        let m1 = s.post_to_channel("s1", "user", None, "text", "do the thing", None)
            .await.unwrap();
        let m2 = s.post_to_channel("s1", "participant", Some("brian"), "text", "done",
                                   Some(Envelope::phase("Apply"))).await.unwrap();

        // Rain has read nothing yet, so both are unread — including the message
        // she was not "forwarded". Context completeness is structural.
        let unread = s.unread_for_participant(r).await.unwrap().rows;
        assert_eq!(unread.len(), 2);
        assert_eq!(unread[0].id, m1.message_id());
        assert_eq!(unread[0].origin, "user");
        assert_eq!(unread[1].id, m2.message_id());
        assert_eq!(unread[1].participant_id, Some(b), "attributed to its author");
        assert_eq!(
            unread[1].envelope.as_ref(),
            Some(&Envelope::phase("Apply")),
            "the envelope is a visible field, not string mutation"
        );

        // After reading, the cursor advances and the backlog empties — and the
        // record of what she was handed goes down in the same commit.
        s.commit_delivery(r, &[(m1.message_id(), None), (m2.message_id(), None)])
            .await
            .unwrap();
        assert!(s.unread_for_participant(r).await.unwrap().rows.is_empty());

        // Brian, who never read, still has the user's message — cursors are per
        // participant. He does NOT have his own row back: he wrote it, which is
        // as read as a message gets.
        let brians = s.unread_for_participant(b).await.unwrap().rows;
        assert_eq!(brians.len(), 1);
        assert_eq!(brians[0].id, m1.message_id());
    }

    #[tokio::test]
    async fn a_channel_read_is_bounded_and_says_when_it_stopped_short() {
        // `channel_after` had no LIMIT, so a participant that had never read
        // got the whole session history in one `Vec` and then onto one wire.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let mut ids = Vec::new();
        for n in 0..5 {
            ids.push(
                s.post_to_channel("s1", "user", None, "text", format!("m{n}"), None)
                    .await
                    .unwrap()
                    .message_id(),
            );
        }

        let page = s.channel_after("s1", 0, 2).await.unwrap();
        assert_eq!(page.rows.iter().map(|m| m.id).collect::<Vec<_>>(), ids[..2]);
        assert!(page.more, "two of five — the caller must be told to come back");

        // And resuming from the last row returned reaches the end, where `more`
        // goes false. Truncating without that flag would make "caught up" and
        // "cut short" the same answer.
        let rest = s.channel_after("s1", ids[1], 100).await.unwrap();
        assert_eq!(rest.rows.iter().map(|m| m.id).collect::<Vec<_>>(), ids[2..]);
        assert!(!rest.more, "that's all");

        // An exact fit is not "more": the probe row is what distinguishes them.
        let exact = s.channel_after("s1", 0, 5).await.unwrap();
        assert_eq!(exact.rows.len(), 5);
        assert!(!exact.more);

        // The two ends of the range, because both are ways a bad number would
        // quietly restore the unbounded read. SQLite reads a NEGATIVE limit as
        // no limit at all, so it is clamped to zero rather than passed through…
        let none = s.channel_after("s1", 0, 0).await.unwrap();
        assert!(none.rows.is_empty());
        assert!(none.more, "zero rows read is not zero rows waiting");
        assert_eq!(s.channel_after("s1", 0, -5).await.unwrap(), none);
        // …and the largest legal limit must not overflow the `limit + 1` probe.
        let everything = s.channel_after("s1", 0, i64::MAX).await.unwrap();
        assert_eq!(everything.rows.len(), 5);
        assert!(!everything.more);
    }

    #[tokio::test]
    async fn a_backlog_is_capped_at_the_batch_limit() {
        // The unbounded case the cap exists for: a participant that has never
        // read a session with more history than one turn should carry.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.ensure_session_roster("s1").await.unwrap();
        let rain = s.participant_by_slug("s1", "rain").await.unwrap().unwrap().id;
        for n in 0..UNREAD_BATCH_LIMIT + 1 {
            s.post_to_channel("s1", "user", None, "text", format!("m{n}"), None)
                .await
                .unwrap();
        }

        let first = s.unread_for_participant(rain).await.unwrap();
        assert_eq!(first.rows.len() as i64, UNREAD_BATCH_LIMIT);
        assert!(first.more, "one row over the cap is one row still owed");

        // Committing the batch is what makes the next call return the rest.
        let batch: Vec<(i64, Option<&str>)> =
            first.rows.iter().map(|m| (m.id, None)).collect();
        s.commit_delivery(rain, &batch).await.unwrap();
        let second = s.unread_for_participant(rain).await.unwrap();
        assert_eq!(second.rows.len(), 1);
        assert!(!second.more);
    }

    #[tokio::test]
    async fn a_participant_does_not_read_its_own_rows_back() {
        // `unread_for_participant` was `channel_after` from the cursor with no
        // author filter at all, so a participant handed its backlog read its
        // OWN last turn back as fresh input — while the doc said "what this
        // participant has not read yet". A participant has, by definition,
        // read what it wrote.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.ensure_session_roster("s1").await.unwrap();
        let brian = s.participant_by_slug("s1", "brian").await.unwrap().unwrap().id;
        let rain = s.participant_by_slug("s1", "rain").await.unwrap().unwrap().id;

        let by_brian = s
            .post_to_channel("s1", "participant", Some("brian"), "text", "my turn", None)
            .await
            .unwrap();
        let by_rain = s
            .post_to_channel("s1", "participant", Some("rain"), "text", "review", None)
            .await
            .unwrap();
        let by_user = s
            .post_to_channel("s1", "user", None, "text", "carry on", None)
            .await
            .unwrap();
        let by_host = s
            .post_to_channel("s1", "system", None, "system_notice", "[System: note]", None)
            .await
            .unwrap();
        // An `origin = 'participant'` row that resolved to nobody — an agent
        // that posted before its roster existed. Unattributable, so it cannot
        // be anyone's own, and dropping it would lose it from every backlog.
        let orphan = s
            .post_to_channel("s1", "participant", Some("nobody"), "text", "orphan", None)
            .await
            .unwrap();

        let unread: Vec<i64> = s
            .unread_for_participant(brian)
            .await
            .unwrap()
            .rows
            .iter()
            .map(|m| m.id)
            .collect();
        assert!(
            !unread.contains(&by_brian.message_id()),
            "a participant must not be handed its own turn as fresh input"
        );
        assert_eq!(
            unread,
            vec![
                by_rain.message_id(),
                by_user.message_id(),
                by_host.message_id(),
                orphan.message_id(),
            ],
            "the peer's turn, the user's message, the host's notice and an \
             unattributed row are all still delivered"
        );

        // Rain's backlog is the mirror image — the filter is per participant,
        // not a blanket "drop participant rows".
        let rain_unread: Vec<i64> = s
            .unread_for_participant(rain)
            .await
            .unwrap()
            .rows
            .iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(
            rain_unread,
            vec![
                by_brian.message_id(),
                by_user.message_id(),
                by_host.message_id(),
                orphan.message_id(),
            ]
        );
    }

    #[tokio::test]
    async fn post_to_channel_resolves_the_participant_by_slug() {
        // Pins the RESOLUTION RULE itself, so moving it out of a separate
        // awaited `participant_by_slug` SELECT and into the INSERT's own
        // subquery is provably behaviour-preserving: this passes on both sides
        // of that change. A test that only passed afterwards would be pinning
        // the refactor rather than the behaviour.
        //
        // Five cases, because each is a way the resolution can be wrong:
        // a known slug resolves; an unknown one degrades to NULL and STILL
        // writes the row (an agent's output vanishing is far worse than one
        // that is unattributed); resolution is scoped to the session, not
        // global; a non-participant origin never resolves at all; and
        // `("participant", None)` resolves to NULL.
        //
        // That last case is the one whose MECHANISM changed. The old form
        // matched it in Rust — `("participant", Some(slug))` was the only arm
        // that looked anything up, so a bare `None` fell to `_ => None`. The
        // new form rests on SQL three-valued logic instead: `slug = NULL` is
        // NULL, never true, so the subquery matches no row and yields NULL.
        // Same answer, different machinery, and nothing in the signature stops
        // a caller writing it — so it is pinned rather than left to be
        // re-derived.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.create_session("s2", "t", None).await.unwrap();
        s.ensure_session_roster("s1").await.unwrap();
        s.ensure_session_roster("s2").await.unwrap();
        let brian1 = s.participant_by_slug("s1", "brian").await.unwrap().unwrap().id;
        let brian2 = s.participant_by_slug("s2", "brian").await.unwrap().unwrap().id;
        assert_ne!(brian1, brian2, "precondition: two rosters, two 'brian' rows");

        let known = s
            .post_to_channel("s1", "participant", Some("brian"), "text", "mine", None)
            .await
            .unwrap();
        let unknown = s
            .post_to_channel("s1", "participant", Some("nobody"), "text", "orphan", None)
            .await
            .unwrap();
        let system = s
            .post_to_channel("s1", "system", Some("brian"), "system_notice", "notice", None)
            .await
            .unwrap();
        let slugless = s
            .post_to_channel("s1", "participant", None, "text", "anonymous", None)
            .await
            .unwrap();
        let elsewhere = s
            .post_to_channel("s2", "participant", Some("brian"), "text", "hers", None)
            .await
            .unwrap();

        let rows = all_rows(&s, "s1").await;
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].id, known.message_id());
        assert_eq!(rows[0].participant_id, Some(brian1), "a known slug resolves");
        assert_eq!(rows[1].id, unknown.message_id());
        assert_eq!(rows[1].participant_id, None, "an unknown slug resolves to NULL");
        assert_eq!(rows[1].content, "orphan", "and the row is written regardless");
        assert_eq!(rows[2].id, system.message_id());
        assert_eq!(
            rows[2].participant_id, None,
            "only a participant origin resolves, even handed a live slug"
        );
        assert_eq!(rows[3].id, slugless.message_id());
        assert_eq!(
            rows[3].participant_id, None,
            "a participant origin with no slug resolves to NULL — `slug = NULL` \
             is NULL, not a match"
        );
        assert_eq!(rows[3].content, "anonymous", "and that row is written too");

        let other = all_rows(&s, "s2").await;
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].id, elsewhere.message_id());
        assert_eq!(
            other[0].participant_id,
            Some(brian2),
            "resolution is scoped to the posting session, not the slug globally"
        );
    }

    #[tokio::test]
    async fn the_legacy_write_path_now_populates_the_new_columns() {
        // B4a dual-write: `insert_message` keeps its signature and its `author`
        // column, and ALSO fills participant_id/origin. That means the channel
        // is correct from the moment 0044 applies — no backfill pass later, and
        // no flag day where attribution is half-migrated.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        let b = s
            .insert_participant("s1", "brian", "Brian", Some(hands.id), None,
                                &hands.capabilities, "active", 0)
            .await
            .unwrap();

        s.insert_message("s1", Author::Brian, MessageKind::Text, "work").await.unwrap();
        s.insert_message("s1", Author::User, MessageKind::Text, "reply").await.unwrap();

        let rows = all_rows(&s, "s1").await;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].participant_id, Some(b), "resolved to the roster entry");
        assert_eq!(rows[0].origin, "participant");
        assert_eq!(rows[1].participant_id, None, "a user message has no participant");
        assert_eq!(rows[1].origin, "user");
    }

    #[tokio::test]
    async fn a_message_from_an_agent_with_no_roster_entry_still_writes() {
        // A session whose roster does not exist yet (created before its
        // participants are inserted) must not fail to log. The subquery
        // resolves to NULL and `author` still carries the attribution —
        // degraded, not broken. Getting this wrong would mean an agent's output
        // vanishing rather than being mis-attributed, which is far worse.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let id = s
            .insert_message("s1", Author::Rain, MessageKind::Text, "no roster yet")
            .await
            .unwrap()
            .message_id();
        assert!(id > 0, "logging must never depend on the roster existing");
        let rows = all_rows(&s, "s1").await;
        assert_eq!(rows[0].participant_id, None);
        assert_eq!(rows[0].origin, "participant");
        // The legacy path still attributes it correctly.
        let legacy = s.messages_for_session("s1", None).await.unwrap();
        assert_eq!(legacy[0].author, "rain");
    }

    #[tokio::test]
    async fn host_injections_become_visible_rows() {
        // The six invisible wires (apply-entry nudge, reconcile directive, idle
        // nudge, phase notices, peer prefix, spawn prompt) post as `system`.
        // Legacy readers must not meet a new author value, so a system row is
        // stored as author='user' — which is exactly how today's system notices
        // are already persisted.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let pm = s
            .post_to_channel("s1", "system", None, "system_notice",
                             "[System: your previous turn was force-interrupted]", None)
            .await
            .unwrap();
        let rows = all_rows(&s, "s1").await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, pm.message_id());
        assert_eq!(rows[0].origin, "system");
        assert!(rows[0].participant_id.is_none());
        // And the legacy read path still sees it, unchanged.
        let legacy = s.messages_for_session("s1", None).await.unwrap();
        assert_eq!(legacy.len(), 1);
        assert_eq!(legacy[0].author, "user", "legacy readers meet no new author value");
    }

    #[tokio::test]
    async fn a_new_session_gets_the_default_roster() {
        // 0044 backfilled only what existed when it applied. Without this,
        // every session created afterwards runs with an empty roster.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.set_session_spawn_config("s1", true, Some("opus"), Some("sonnet")).await.unwrap();

        assert_eq!(s.ensure_session_roster("s1").await.unwrap(), 2);

        let roster = s.participants_for_session("s1").await.unwrap();
        assert_eq!(roster.len(), 2);
        assert_eq!(roster[0].slug, "brian");
        assert_eq!(roster[0].turn_position, 0, "HANDS acts first");
        assert_eq!(roster[0].model_id.as_deref(), Some("opus"), "model snapshotted off the row");
        assert!(roster[0].capabilities.contains("edit_files"));
        assert!(roster[0].enabled);
        assert_eq!(roster[1].slug, "rain");
        assert_eq!(roster[1].turn_position, 1);
        assert_eq!(roster[1].model_id.as_deref(), Some("sonnet"));
        assert!(!roster[1].capabilities.contains("edit_files"), "EYES stays read-only");
        // A participant without a cursor is invisibly undeliverable.
        for p in &roster {
            assert_eq!(s.cursor_for(p.id).await.unwrap(), 0);
        }
    }

    #[tokio::test]
    async fn seeding_a_roster_twice_is_a_no_op() {
        // It runs pre-spawn on EVERY respawn, so non-idempotence would mean a
        // duplicate roster per restart.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        assert_eq!(s.ensure_session_roster("s1").await.unwrap(), 2);
        assert_eq!(s.ensure_session_roster("s1").await.unwrap(), 0, "second call inserts nothing");
        assert_eq!(s.participants_for_session("s1").await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_solo_session_keeps_rain_disabled() {
        // Same shape 0044 gave the 12 solo sessions it backfilled: the row
        // exists (so promoting later is an UPDATE, not an invite) but is off.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.set_session_spawn_config("s1", false, None, None).await.unwrap();
        assert_eq!(s.ensure_session_roster("s1").await.unwrap(), 2);
        let roster = s.participants_for_session("s1").await.unwrap();
        assert!(roster[0].enabled, "HANDS runs");
        assert!(!roster[1].enabled, "EYES present but disabled");
        assert!(s.next_active_participant("s1", None).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn seeding_repairs_messages_written_before_the_roster() {
        // The live defect: a post-0044 session logged 60 messages with
        // participant_id NULL before anything created its roster. Seeding must
        // map them, or that history is permanently unattributed in the channel.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.insert_message("s1", Author::Brian, MessageKind::Text, "work").await.unwrap();
        s.insert_message("s1", Author::Rain, MessageKind::Text, "review").await.unwrap();
        s.insert_message("s1", Author::User, MessageKind::Text, "reply").await.unwrap();
        let before = all_rows(&s, "s1").await;
        assert!(before.iter().all(|m| m.participant_id.is_none()), "precondition: unmapped");

        s.ensure_session_roster("s1").await.unwrap();

        let roster = s.participants_for_session("s1").await.unwrap();
        let after = all_rows(&s, "s1").await;
        assert_eq!(after[0].participant_id, Some(roster[0].id), "brian's row mapped");
        assert_eq!(after[1].participant_id, Some(roster[1].id), "rain's row mapped");
        assert_eq!(after[2].participant_id, None, "a user row has no participant");
        assert_eq!(after[2].origin, "user");
    }

    #[tokio::test]
    async fn insert_message_resolves_the_participant_once_the_roster_exists() {
        // Closes the loop with B4a's dual-write: seeded roster → the inline
        // subquery resolves, so nothing new accumulates unmapped.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.ensure_session_roster("s1").await.unwrap();
        s.insert_message("s1", Author::Brian, MessageKind::Text, "work").await.unwrap();
        let roster = s.participants_for_session("s1").await.unwrap();
        let rows = all_rows(&s, "s1").await;
        assert_eq!(rows[0].participant_id, Some(roster[0].id));
        assert_eq!(rows[0].origin, "participant");
    }

    #[tokio::test]
    async fn a_persisted_message_carries_the_row_it_came_from() {
        // B5 Task 2 renders the wire FROM the receipt and never re-reads the
        // row, so the assertions below compare the receipt against the
        // PERSISTED row rather than against the arguments it was built from.
        // Checking it against the arguments would pass by construction; the
        // property that matters is that the two cannot diverge, because a
        // divergence makes the row a lie — the user's record and the agent's
        // actual input would silently disagree, which is the exact failure
        // this type exists to rule out.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.ensure_session_roster("s1").await.unwrap();
        let pm = s
            .post_to_channel("s1", "participant", Some("brian"), "text", "work",
                             Some(Envelope::phase("Apply")))
            .await
            .unwrap();
        assert!(pm.message_id() > 0, "a PersistedMessage is proof of a row");

        let rows = all_rows(&s, "s1").await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, pm.message_id());
        // Against the ROW, not the argument: Task 2's cross-session guard rests
        // on the receipt naming the channel the row actually landed in.
        assert_eq!(
            pm.session_id(),
            rows[0].session_id,
            "a receipt is scoped to the channel it was persisted into"
        );
        assert_eq!(pm.body(), rows[0].content, "receipt body IS the persisted body");
        assert_eq!(
            pm.envelope(),
            rows[0].envelope.as_ref(),
            "receipt envelope IS the persisted envelope"
        );
        // And therefore the wire too — the receipt renders the same bytes the
        // stored row does, which is the whole of what a delivery may write.
        assert_eq!(
            pm.wire(),
            render_wire(rows[0].envelope.as_ref(), &rows[0].content)
        );
    }

    #[tokio::test]
    async fn a_receipt_for_a_row_read_back_renders_the_same_wire() {
        // The sequencer's path: rows come out of `channel_after` /
        // `unread_for_participant` as `ChannelMessage`, long after the
        // `PersistedMessage` the INSERT minted has been dropped — after a
        // restart there never was one in this process. `from_row` is what lets
        // those be delivered without reopening `deliver` to strings.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let posted = s
            .post_to_channel(
                "s1",
                "system",
                None,
                MessageKind::SystemNotice.as_str(),
                "declare state",
                Some(Envelope::phase("Verify").with_open_blocking(3)),
            )
            .await
            .unwrap();

        let row = &all_rows(&s, "s1").await[0];
        let replayed = PersistedMessage::from_row(row);

        // Same row, so the same bytes on stdin — the receipt read back is worth
        // exactly what the one from the INSERT was worth.
        assert_eq!(replayed.message_id(), posted.message_id());
        assert_eq!(replayed.body(), posted.body());
        assert_eq!(replayed.envelope(), posted.envelope());
        assert_eq!(replayed.wire(), posted.wire());
        assert_eq!(
            replayed.wire(),
            "[PHASE: Verify]\n⚠ 3 unresolved EYES blocking finding(s) — run \
             check_open_findings and disposition each (fix/rebut) before you \
             commit.\ndeclare state"
        );
        // The scope survives the round trip, or `send_to_all`'s check would wave
        // every replayed row through.
        assert_eq!(replayed.session_id(), "s1");
    }

    #[tokio::test]
    async fn a_session_with_no_active_participants_is_already_done() {
        // An all-observer session must not wedge the sequencer on an unwrap —
        // and must not wedge it on a SPIN either, which is what the two answers
        // used to add up to. `next_active_participant` said `None` (no turn to
        // hand out) and `all_active_voted_done` said `false` (not done), so a
        // loop reading the pair had nothing to do and no reason to stop.
        //
        // The pair is now coherent, and the shape is: an empty rotation is
        // DONE. Vacuously — every active participant has voted done when there
        // are none — and usefully, because nobody left can produce output, so
        // waiting is waiting on nothing. The sequencer halts and waits for a
        // wake (a user message, a participant being enabled) rather than
        // cycling. Reached by an all-observer or all-`on_demand` roster, by
        // every active participant being disabled, and by a session with no
        // roster yet — all four covered below or by the roster tests.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.insert_participant("s1", "o", "O", None, None, "[]", "observer", 0).await.unwrap();
        // `on_demand` too — skipped in the rotation, woken only when addressed,
        // so a roster of nothing but these is also an empty rotation.
        s.insert_participant("s1", "d", "D", None, None, "[]", "on_demand", 1).await.unwrap();
        assert!(s.next_active_participant("s1", None).await.unwrap().is_none());
        assert!(
            s.all_active_voted_done("s1").await.unwrap(),
            "no actives = nothing left to wait for"
        );

        // A session with no roster at all is the same answer for the same
        // reason — and it is reachable, since `ensure_session_roster` only runs
        // pre-spawn.
        s.create_session("s2", "t", None).await.unwrap();
        assert!(s.next_active_participant("s2", None).await.unwrap().is_none());
        assert!(s.all_active_voted_done("s2").await.unwrap());

        // Disabling the last active participant is the same state, and it is the
        // one a user can reach from the UI rather than by building an
        // all-observer roster.
        let a = s
            .insert_participant("s1", "a", "A", None, None, "[]", "active", 1)
            .await
            .unwrap();
        assert!(s.next_active_participant("s1", None).await.unwrap().is_some());
        assert!(!s.all_active_voted_done("s1").await.unwrap());
        sqlx::query("UPDATE session_participants SET enabled = 0 WHERE id = ?")
            .bind(a)
            .execute(s.pool())
            .await
            .unwrap();
        assert!(s.next_active_participant("s1", None).await.unwrap().is_none());
        assert!(s.all_active_voted_done("s1").await.unwrap());

        // But the implication runs ONE WAY, and this is the trap the sequencer
        // must not fall into. Re-enable A and vote it done: consensus is `true`
        // while there is still a turn to hand out, so `is_none()` is NOT a halt
        // test — it would never fire in a session that has participants in it.
        sqlx::query("UPDATE session_participants SET enabled = 1 WHERE id = ?")
            .bind(a)
            .execute(s.pool())
            .await
            .unwrap();
        s.set_done_vote(a, true).await.unwrap();
        assert!(s.all_active_voted_done("s1").await.unwrap(), "consensus reached");
        assert!(
            s.next_active_participant("s1", None).await.unwrap().is_some(),
            "the ring is unchanged by how anyone voted — done is not the same \
             condition as having no next turn"
        );
    }
}
