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
    /// `active` | `on_mention` — see [`PARTICIPATION_MODES`].
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

/// The participation modes, as data. **Two, and both of them do something**
/// (rc3 D18).
///
/// `active` is in the rotation. `on_mention` is not: it is spawned, skipped by
/// the ring, and handed a turn only when the USER names it — one turn, then out
/// again (rc3 D17).
///
/// A guard rather than documentation because the value is compared as a STRING
/// with no CHECK constraint behind it: `next_active_participant` filters the
/// ring on `p.participation_mode == "active"`, so a role stored as `"Active"`
/// or `"actve"` produces participants that are enabled, visible in the roster,
/// counted by `all_active_voted_done` — no, not even counted, since that filters
/// on the same string — and simply never given a turn. The failure is a session
/// that looks fully staffed and never advances, with nothing to grep for.
///
/// **`observer` was the third and is gone** (rc3 D18). It was spawned, handed no
/// turn, delivered nothing and could not vote — a subprocess that read nothing,
/// said nothing and billed for existing. Its one defensible use, a role that
/// watches and speaks rarely, is what `on_mention` is.
pub const PARTICIPATION_MODES: [&str; 2] = ["active", "on_mention"];

/// The mode a participant must be in to sit in the turn rotation.
///
/// Named because three places filter on it — the ring read, the vote tally and
/// 0045's partial index — and a fourth would be written as a bare `"active"`.
pub const MODE_ACTIVE: &str = "active";

/// The mode that waits to be summoned. See [`PARTICIPATION_MODES`].
pub const MODE_ON_MENTION: &str = "on_mention";

/// How many participants one session may run.
///
/// **Declared here, beside the roster invariant, rather than in the command
/// layer** (round-2 audit B3). It used to live in `tauri_cmd::sessions` and be
/// enforced in `resolve_participant_picks` — the create DIALOG's path, one of
/// three. The other two seed instead of picking, through
/// [`Storage::ensure_session_roster`], and that path had no ceiling: a plugin
/// or the external driver got every active non-`on_mention` role, however many
/// existed. A limit enforced on one path of three is a limit on none of them.
///
/// Not the runtime limit — rc3 D10 made spawn iterate the roster, so this is a
/// sanity bound on what one session can usefully run. Every participant is a
/// claude-code subprocess with its own context window and its own bill.
pub const MAX_SESSION_PARTICIPANTS: usize = 8;

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
    /// Per-participant spawn knobs (rc3 **D12**). `None` = inherit, exactly as
    /// the `sessions.{brian,rain}_effort` / `_ultracode` columns they replace
    /// meant it. Columns since 0044; only rc3 reads them.
    pub effort: Option<String>,
    pub ultracode: Option<bool>,
    /// This participant's prior claude-code conversation id, so a respawn
    /// resumes instead of starting blank. Was `sessions.{brian,rain}_claude_session_id`.
    pub claude_session_id: Option<String>,
    /// The user's colour pick for this participant, by palette NAME, or `None`
    /// to take the rotation (rc3 D20).
    pub color: Option<String>,
    /// The user's NAME for this participant, or `None`/blank to take the
    /// ordinal (rc3 D20, migration 0053). See
    /// [`participant_display_name`] for what it overrides and what it leaves
    /// alone.
    pub label: Option<String>,
}

/// One participant a session is created with: **a role and a model**.
///
/// The two things rc3's New Session dialog picks per row, plus the two spawn
/// knobs the dialog already had. It is the input to
/// [`Storage::seed_session_roster`], which is the N-participant counterpart of
/// [`Storage::ensure_session_roster`].
///
/// **There is no name here, and that is rc3 D10.** The slug and the display
/// name are DERIVED from the role by [`Storage::seed_session_roster`] — see
/// [`participant_slug`] for how a second participant of the same role is kept
/// addressable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParticipantDraft {
    pub role_id: i64,
    /// rc3 **D8**: `None` falls back to the role's `default_model_id`. The
    /// dialog's per-participant picker is the override.
    pub model_id: Option<String>,
    /// Per-participant spawn knobs, mirroring the columns 0044 gave the table.
    /// `None` = inherit, exactly as the `sessions.{brian,rain}_effort` columns
    /// these generalise mean it.
    pub effort: Option<String>,
    pub ultracode: Option<bool>,
    /// The palette entry the user picked for this participant, by NAME
    /// ("Cyan"), or `None` to take the rotation (rc3 **D20**, migration 0052).
    pub color: Option<String>,
    /// The name the user gave this participant, or `None` to take the ordinal
    /// (rc3 **D20**, migration 0053).
    pub label: Option<String>,
}

const ROLE_COLUMNS: &str = "id, slug, display_name, description_prompt, capabilities, \
     participation_mode, default_model_id, builtin, archived";

const PARTICIPANT_COLUMNS: &str = "id, session_id, slug, display_name, role_id, model_id, \
     runtime, capabilities, participation_mode, turn_position, done_vote, enabled, \
     effort, ultracode, claude_session_id, color, label";

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
        effort: r.get("effort"),
        // `<> 0` for the same reason `enabled` is decoded that way — the column
        // is a nullable INTEGER with no CHECK, so anything storable is truthy.
        ultracode: r.get::<Option<i64>, _>("ultracode").map(|v| v != 0),
        claude_session_id: r.get("claude_session_id"),
        color: r.get("color"),
        label: r.get("label"),
    }
}

/// The ring step, as a pure function of the rotation.
///
/// Split out of [`Storage::next_active_participant`] so the scheduling rule can
/// be exercised over rosters the database will not produce — which is the only
/// way to test what the ring does with a roster the schema once permitted.
///
/// `ring` is the active participants in `(turn_position, id)` order and nothing
/// else; `on_mention` and disabled rows are filtered out by the caller, because
/// a wake nobody asked for is pure waste. An `on_mention` participant is reached
/// by being SUMMONED — the sequencer hands it a turn directly (rc3 D17) — never
/// by this step.
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
/// `on_mention` — which is what "let the summoned one stay in the rotation"
/// would mean — is the same trap, one line away.
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
        // `on_mention` while it held the turn — or it IS an `on_mention`
        // participant that was summoned — so there is no place to step one
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
/// The per-session handle a participant playing `role_slug` takes, given the
/// handles its session has already handed out (rc3 **D10**).
///
/// **How collisions are resolved:** the role's own slug is the handle, and the
/// SECOND participant of that role in the same session takes `<role>-2`, the
/// third `<role>-3`, and so on — [`first_free_slug`]'s rule, reused verbatim so
/// a participant handle and a role slug are suffixed by one function rather than
/// by two that can disagree. `taken` must therefore hold every handle already
/// allocated for THIS session (`UNIQUE (session_id, slug)` is the constraint
/// behind it), not the role slugs.
///
/// Two participants of the same role are both addressable because of this: the
/// slug is what `@mention` parses and what `participant_by_slug` looks up, so a
/// pair that shares a role, a model and a display name is still two distinct
/// handles. Nothing here is keyed on an agent NAME — it never was the role's
/// display name and it is no longer a person's.
pub fn participant_slug(role_slug: &str, taken: &HashSet<String>) -> String {
    // A role whose slug is empty cannot happen through `create_role` (it
    // slugifies, and `slugify` falls back to `role`), but the column has no
    // CHECK, so a hand-edited row is possible and an empty handle would be
    // unaddressable rather than merely odd.
    let base = if role_slug.trim().is_empty() {
        FALLBACK_SLUG
    } else {
        role_slug
    };
    first_free_slug(base, taken)
}

/// **The display rule, in one place** (rc3 D10, and the binding contract between
/// the backend and the frontend for this phase):
///
/// > `role_display_name · model_display_name`, e.g. `HANDS · Claude Opus 5`.
/// > When `role_display_name` is null, fall back to the model alone; when both
/// > are null, fall back to the slug.
///
/// **A participant is NEVER displayed as "Brian" or "Rain".** The name a human
/// sees is the role it plays and the model it runs on, and the user renames
/// either of those whenever they like.
///
/// The slug fallback is the last resort rather than a nicety: `roles.display_name`
/// is `NOT NULL`, so a `None` role means the row is GONE (archived rows keep
/// their display name, so even those still render), and a participant with no
/// model and no role would otherwise render as an empty string in a list.
pub fn participant_display_name(
    role_display_name: Option<&str>,
    model_display_name: Option<&str>,
    slug: &str,
    label: Option<&str>,
) -> String {
    fn clean(s: Option<&str>) -> Option<&str> {
        s.map(str::trim).filter(|s| !s.is_empty())
    }
    // **The label replaces the role-and-ordinal half, and only that half** (rc3
    // D20, migration 0053). The model suffix survives it, because what a
    // participant RUNS is a different fact from what the user named it — a
    // `Skeptic` whose model the user cannot see is the thing D8's per-participant
    // picker exists to make visible.
    //
    // Blank is not a name: an empty or whitespace label falls back to the
    // ordinal rather than rendering an empty byline, which is the same `clean`
    // every other field on this path goes through.
    let role = clean(label).map(str::to_string).or_else(|| {
        clean(role_display_name).map(|role| match slug_ordinal(slug) {
            Some(n) => format!("{role}-{n}"),
            None => role.to_string(),
        })
    });
    match (role, clean(model_display_name)) {
        (Some(role), Some(model)) => format!("{role} · {model}"),
        (Some(role), None) => role,
        (None, Some(model)) => model.to_string(),
        (None, None) => slug.to_string(),
    }
}

/// The `-N` a duplicate slug carries, if any: `eyes-2` → `Some(2)`, `eyes` →
/// `None` (rc3 **D20**).
///
/// **Two participants of one role rendered identically, character for
/// character**, which is what the user reported after a live N=3 run: *"for the
/// 2 reviewers, i don't know which is which."* `EYES · DeepSeek V4 Pro` twice,
/// in the roster, in the chat bylines, and in the same colour — because the
/// display rule had no ordinal and the colour is hashed from the label.
///
/// The ordinal is taken from the SLUG rather than counted over the roster, and
/// that is the point rather than a shortcut: `first_free_slug` already assigns
/// `eyes`, `eyes-2`, `eyes-3` at invite time, so the visible name and the
/// internal key agree by construction and cannot drift. A count over the roster
/// would be a second numbering, and two numberings of one thing disagree the
/// first time a participant is disabled.
///
/// The first of a role takes no suffix, which is why a session with ONE reviewer
/// still reads `EYES` and nothing changes for the common case.
///
/// Conservative about what counts: only a trailing `-` followed by digits, and
/// only when something precedes it. A role legitimately named `Agent-7` slugs to
/// `agent-7` and would read as `AGENT-7` — the same string either way, so the
/// worst case is a suffix that was already there.
fn slug_ordinal(slug: &str) -> Option<u32> {
    let (base, tail) = slug.rsplit_once('-')?;
    if base.is_empty() {
        return None;
    }
    tail.parse().ok().filter(|n| *n >= 2)
}

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
/// which is a legal configuration and reads as intentional. A
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

    /// Load a role by its primary key — the id a participant row carries in
    /// `role_id`.
    ///
    /// Exists so the spawn path can resolve a participant's role prose WITHOUT
    /// mapping an agent name onto a role slug. `role_by_slug("hands")` would
    /// have re-introduced the `agent == "hands"` coupling that 0044 exists to
    /// remove, and would be wrong the moment a user renames a role or adds a
    /// third one. `role_id` is the participant's own answer to "which role am
    /// I", so it stays right under both.
    pub async fn role_by_id(&self, id: i64) -> Result<Option<Role>> {
        let row = sqlx::query(&format!("SELECT {ROLE_COLUMNS} FROM roles WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .with_context(|| format!("loading role id {id}"))?;
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
        // `updated_at` bound explicitly, NOT left to the column default
        // (round-2 audit R5). The default is `datetime('now')`, which SQLite
        // emits zone-less (`2026-08-12 15:48:26`) while every other time in
        // this database is RFC3339-Z — and this insert omitted the column, so
        // the default fired on every cursor ever created: 85 of 90 rows in the
        // live database were zone-less, and only the ones a later `UPDATE` had
        // touched were right. A zone-less stamp sorts BEFORE any same-day
        // RFC3339 one (`' '` 0x20 < `'T'` 0x54), and the frontend parses it as
        // LOCAL time, which is the staleness hallucination `storage::time`
        // exists to prevent.
        sqlx::query("INSERT INTO participant_cursors (participant_id, updated_at) VALUES (?, ?)")
            .bind(id)
            .bind(crate::storage::now_utc())
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

    /// A participant by the token a user typed after `@` — its slug, or the
    /// name the user gave it (rc3 **D20**, migration 0053).
    ///
    /// **The slug is tried first and wins outright**, because it is the key: it
    /// is assigned at invite time, it is unique per session by constraint, and
    /// nothing the user types later can change it. A label is a preference, and
    /// a preference that could shadow a key would let renaming one participant
    /// silently redirect summons meant for another.
    ///
    /// Labels are matched case-folded and trimmed, the same normalisation
    /// [`crate::core::mentions::parse_mention_slugs`] already applies to the
    /// token — a user who typed `@Skeptic` meant the participant called
    /// `Skeptic`. `lower()` is ASCII-only in SQLite, which is exactly the range
    /// the mention parser accepts, so the two cannot disagree about what a
    /// match is.
    ///
    /// Two participants sharing a label is not prevented (the column is
    /// deliberately unvalidated) and is resolved by turn order — the earliest
    /// seat. Arbitrary, but stable and predictable, which is what D1 asks of
    /// anything that decides who acts next.
    pub async fn participant_by_mention(
        &self,
        session_id: &str,
        token: &str,
    ) -> Result<Option<Participant>> {
        if let Some(p) = self.participant_by_slug(session_id, token).await? {
            return Ok(Some(p));
        }
        let row = sqlx::query(&format!(
            "SELECT {PARTICIPANT_COLUMNS} FROM session_participants \
             WHERE session_id = ? AND lower(trim(label)) = ? \
             ORDER BY turn_position, id LIMIT 1"
        ))
        .bind(session_id)
        .bind(token.trim().to_ascii_lowercase())
        .fetch_optional(&self.pool)
        .await
        .context("loading participant by label")?;
        Ok(row.as_ref().map(participant_from_row))
    }

    /// One participant's name, by the display rule
    /// ([`participant_display_name`]): the ROLE it plays and the MODEL it runs
    /// on, never a person's name (rc3 D10).
    ///
    /// Both halves are read live rather than off the row's frozen
    /// `display_name`, so renaming a role or swapping a model is reflected
    /// without waiting for a respawn. Every failure — no `role_id`, an
    /// archived-away role, a deleted model, a query error — degrades ONE half to
    /// `None` and the display rule resolves what is left, down to the slug.
    ///
    /// Lives here rather than at either call site because both the spawn path
    /// (`core::session`) and the reviewer's phase-doc header
    /// (`SignalingBridge::session_doc_write_eyes`) name a participant, and two
    /// copies of a display rule are two things that can disagree about what a
    /// participant is called.
    pub async fn display_name_of(&self, p: &Participant) -> String {
        let role = match p.role_id {
            Some(id) => match self.role_by_id(id).await {
                Ok(r) => r.map(|r| r.display_name),
                Err(e) => {
                    tracing::warn!(role_id = id, ?e, "reading a role's display name failed");
                    None
                }
            },
            None => None,
        };
        let model = match p.model_id.as_deref().filter(|m| !m.is_empty()) {
            Some(id) => match self.get_model(id).await {
                Ok(m) => m.map(|m| m.display_name),
                Err(e) => {
                    tracing::warn!(model_id = %id, ?e, "reading a model's display name failed");
                    None
                }
            },
            None => None,
        };
        participant_display_name(role.as_deref(), model.as_deref(), &p.slug, p.label.as_deref())
    }

    /// Seed the DEFAULT roster for a session that has none, returning how many
    /// participants were inserted (0 on the common path).
    ///
    /// Every creation path funnels through `ensure_session_started` → spawn, so
    /// this is what keeps a session created WITHOUT the New Session dialog — the
    /// external driver's `open_session`, the plugin proxy's
    /// `dispatch_session_inner` — from starting life with an empty roster.
    /// Without a roster every message it writes resolves `participant_id` to
    /// NULL forever, because `insert_message`'s dual-write looks the roster up
    /// by slug.
    ///
    /// **rc3 D10: the roster is derived from the user's ROLES, not from two
    /// literal `WHERE slug = 'hands' / 'eyes'` subqueries.** The default is every
    /// live role that takes turns (`archived = 0`, `participation_mode <>
    /// 'on_mention'`), in `roles.id` order — the order the user created them —
    /// with the slug and the display name derived from each role by
    /// [`participant_slug`] and `roles.display_name`. On the seeded pair that is
    /// exactly today's roster in today's turn order (HANDS at slot 0, EYES at
    /// slot 1), which `the_default_roster_is_role_derived_in_creation_order`
    /// pins.
    ///
    /// **`first_role_only` is the PRODUCT DEFAULT for every create path with no
    /// dialog, and it has no UI behind it (rc3 D13).** The
    /// `rain_disabled_default` setting that used to answer this is deleted — the
    /// user's words: *"there is no 'disable the reviewer by default'; just don't
    /// add the role to your session creation"* — so the external driver
    /// (`CoreAppState::open_session`) and the plugin create arm
    /// (`dispatch_session_inner`) now pass `true` and this seeds **exactly one
    /// participant: the first active role by `roles.id`**, per design §1 ("how
    /// many agents, **default 1**"). Anything that wants more picks a roster,
    /// through the New Session dialog or `seed_session_roster`.
    ///
    /// One ROW, not N rows with the extras disabled — which is what this did
    /// while the roster was a fixed pair. A disabled row for a role the creator
    /// never chose is a participant the session view renders and nothing wakes;
    /// under N roles it would be one such row per role the user has ever made.
    ///
    /// **This is the DEFAULT roster, not the only one.** A session created
    /// through [`Storage::seed_session_roster`] already has the roster its
    /// creator chose, and this must not add to it — hence the count guard.
    pub async fn ensure_session_roster(&self, session_id: &str, wanted: usize) -> Result<u64> {
        // Seed only into a session that has NO roster.
        //
        // `OR IGNORE` on `UNIQUE (session_id, slug)` was the whole idempotence
        // story while `brian` + `rain` were the only rows that could exist. It
        // stops being one the moment a roster can hold a different SET of slugs:
        // a session created with one participant would collide on the first
        // insert and sail through the second, silently acquiring a reviewer
        // nobody invited.
        let (existing,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM session_participants WHERE session_id = ?")
                .bind(session_id)
                .fetch_one(&self.pool)
                .await
                .with_context(|| format!("counting the roster of {session_id}"))?;
        if existing > 0 {
            return Ok(0);
        }
        // `roles.id` order, not `list_roles`' `ORDER BY slug`: creation order is
        // stable under a rename, and renaming a role must not reorder who acts
        // first.
        let rows = sqlx::query(&format!(
            "SELECT {ROLE_COLUMNS} FROM roles \
             WHERE archived = 0 AND participation_mode <> '{MODE_ON_MENTION}' ORDER BY id"
        ))
        .fetch_all(&self.pool)
        .await
        .context("reading the roles the default roster is built from")?;
        let roles: Vec<Role> = rows.iter().map(role_from_row).collect();
        if roles.is_empty() {
            // A fresh install with no roles yet. Nothing to seed and nothing to
            // repair; the session runs with no participants rather than with
            // rows whose `role_id` is NULL, which is the failure the two literal
            // subqueries used to produce silently.
            return Ok(0);
        }
        // The cut, applied to BOTH the drafts and the role list they are zipped
        // against — `insert_roster` reads the slug and display name off the
        // matching role, so truncating one without the other would seed a
        // participant under the wrong role's name.
        //
        // **`wanted` is a COUNT, and this was a `first_role_only: bool`**
        // (round-2 audit B3). The boolean had two failures at once. It could not
        // express a roster: `false` meant "every active non-`on_mention` role",
        // so a caller asking for a pair got however many roles the user had
        // configured — three today, silently four the moment one is added in
        // Settings → Roles, and every participant is a claude-code subprocess
        // with its own bill. And it had no ceiling, while the create DIALOG's
        // path refused more than [`MAX_SESSION_PARTICIPANTS`] in
        // `resolve_participant_picks` — a cap on one path of three is a cap on
        // none of them.
        //
        // Clamped, not rejected: this is the SEED path, whose callers are
        // asking for a default rather than naming a roster, and there is
        // nothing useful to fail for when a caller wants more roles than the
        // install has. Asking for more than exists yields what exists.
        let roles: Vec<Role> = roles
            .into_iter()
            .take(wanted.clamp(1, MAX_SESSION_PARTICIPANTS))
            .collect();
        let drafts: Vec<ParticipantDraft> = roles
            .iter()
            .map(|role| ParticipantDraft {
                role_id: role.id,
                ..ParticipantDraft::default()
            })
            .collect();
        let ids = self.insert_roster(session_id, &drafts, &roles, None).await?;
        // Repair what a rosterless window wrote. Only reachable when this call
        // actually inserted, so a healthy respawn never runs it. Scoped to
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
        Ok(ids.len() as u64)
    }

    /// Seed a session's roster from N chosen roles, returning the new
    /// participant ids in the order they were given.
    ///
    /// The dialog's counterpart to [`Storage::ensure_session_roster`]: same
    /// table, same invite-time snapshot, same cursor-from-birth invariant, same
    /// derived handles — the roles come from the caller instead of from the role
    /// table's own order. Nothing about the ROW differs, which is what makes a
    /// two-participant HANDS + EYES session identical to today's (proved by
    /// `n_of_two_is_byte_identical_to_the_default_roster`, which compares every
    /// column of both rosters).
    ///
    /// **Turn slots are the order given**, 0..N.
    /// [`Storage::next_active_participant`] advances by place in that order, and
    /// `core::session::spawn_session_handle` spawns one agent per enabled row in
    /// the same order, so the list IS the turn order.
    ///
    /// **Not idempotent, on purpose.** `ensure_session_roster` runs on every
    /// spawn and must be a no-op on the common path; this runs once, at create,
    /// and a second call means something has gone wrong upstream. Seeding over
    /// an existing roster would double it, so it is an error rather than a merge.
    ///
    /// The whole batch is ONE transaction. Half a roster is worse than none:
    /// `ensure_session_roster`'s count guard will not heal it, and a session
    /// missing its reviewer runs unreviewed rather than failing.
    pub async fn seed_session_roster(
        &self,
        session_id: &str,
        drafts: &[ParticipantDraft],
    ) -> Result<Vec<i64>> {
        if drafts.is_empty() {
            anyhow::bail!("a session needs at least one participant");
        }
        let mut roles = Vec::with_capacity(drafts.len());
        for draft in drafts {
            let role = self
                .role_by_id(draft.role_id)
                .await?
                .with_context(|| format!("role {} does not exist", draft.role_id))?;
            roles.push(role);
        }
        self.insert_roster(session_id, drafts, &roles, None).await
    }

    /// The one INSERT both roster paths run, so a dialog-built roster and a
    /// default one cannot drift apart column by column.
    ///
    /// `roles[i]` is the already-loaded role for `drafts[i]` — passed in rather
    /// than re-read, so the caller's validation (archived? on-demand?) and the
    /// snapshot that gets stored come from one read. `enabled` is `None` for
    /// "every row enabled".
    async fn insert_roster(
        &self,
        session_id: &str,
        drafts: &[ParticipantDraft],
        roles: &[Role],
        enabled: Option<&[bool]>,
    ) -> Result<Vec<i64>> {
        debug_assert_eq!(drafts.len(), roles.len());
        // `joined_at` is the SESSION's `created_at`, not `datetime('now')` — the
        // value 0044's backfill wrote. The double duty is why: it is also the
        // existence check, so a roster cannot be seeded into a session id that
        // has no row (the FK would catch it, as a constraint error naming an
        // integer).
        let created_at: Option<(String,)> =
            sqlx::query_as("SELECT created_at FROM sessions WHERE id = ?")
                .bind(session_id)
                .fetch_optional(&self.pool)
                .await
                .with_context(|| format!("reading session {session_id}"))?;
        let (created_at,) =
            created_at.with_context(|| format!("session {session_id} does not exist"))?;
        let (existing,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM session_participants WHERE session_id = ?")
                .bind(session_id)
                .fetch_one(&self.pool)
                .await
                .with_context(|| format!("counting the roster of {session_id}"))?;
        if existing > 0 {
            anyhow::bail!(
                "session {session_id} already has {existing} participant(s); \
                 a roster is seeded once"
            );
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .context("opening the roster transaction")?;
        let mut ids = Vec::with_capacity(drafts.len());
        // Handles are allocated as the batch is built, so the SECOND participant
        // of a role in this session takes `<role>-2` — see [`participant_slug`].
        let mut taken: HashSet<String> = HashSet::new();
        for (slot, (draft, role)) in drafts.iter().zip(roles).enumerate() {
            let slug = participant_slug(&role.slug, &taken);
            taken.insert(slug.clone());
            // The invite-time snapshot: `capabilities` and `participation_mode`
            // are COPIED off the role, so editing the role later cannot widen a
            // live participant mid-session. `role_id` records which template
            // this came from; these two record what it actually runs with.
            let id = sqlx::query(
                "INSERT INTO session_participants \
                 (session_id, slug, display_name, role_id, model_id, effort, ultracode, \
                  capabilities, participation_mode, turn_position, enabled, joined_at, color, \
                  label) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(session_id)
            .bind(&slug)
            // rc3 D10: the human-facing half of the display rule, snapshotted off
            // the ROLE at invite. The model half is resolved live by whoever
            // renders it (`list_session_participants` returns the two
            // separately) — freezing a model name in this column would leave a
            // stale one on screen the moment the user changes the model.
            .bind(&role.display_name)
            .bind(role.id)
            // D8's fallback: the participant's own pick, else the role's default.
            .bind(draft.model_id.as_deref().or(role.default_model_id.as_deref()))
            .bind(draft.effort.as_deref())
            .bind(draft.ultracode.map(i64::from))
            .bind(&role.capabilities)
            .bind(&role.participation_mode)
            .bind(slot as i64)
            .bind(i64::from(enabled.map(|e| e[slot]).unwrap_or(true)))
            .bind(&created_at)
            .bind(draft.color.as_deref())
            .bind(draft.label.as_deref())
            .execute(&mut *tx)
            .await
            .with_context(|| format!("seeding participant {slug} into {session_id}"))?
            .last_insert_rowid();
            // Every participant reads the channel, so every participant has a
            // cursor from birth — the invariant `insert_participant` also holds.
            // Without it a delivery records itself, leaves the cursor at 0, and
            // re-offers the same batch every turn while reporting success.
            // `updated_at` bound, not defaulted — see `insert_participant`.
            sqlx::query(
                "INSERT INTO participant_cursors (participant_id, updated_at) VALUES (?, ?)",
            )
            .bind(id)
            .bind(crate::storage::now_utc())
            .execute(&mut *tx)
            .await
            .context("seeding participant cursor")?;
            ids.push(id);
        }
        tx.commit().await.context("committing the roster")?;
        Ok(ids)
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
    /// Record who holds the turn, so the UI can say "waiting for its turn"
    /// rather than leaving a participant that has not been reached yet
    /// indistinguishable from a dead one.
    ///
    /// `sessions.current_turn_participant_id` has existed since 0044 and
    /// **nothing wrote it** — the ring knew whose turn it was and the column
    /// that exists to say so stayed NULL. Reported from a live N=3 session where
    /// only the first participant acted for two minutes and read as two broken
    /// agents.
    ///
    /// Best-effort: a failed write costs a UI hint, never a turn, so it warns
    /// and moves on rather than propagating.
    pub async fn set_current_turn(&self, session_id: &str, participant_id: Option<i64>) {
        if let Err(e) = sqlx::query(
            "UPDATE sessions SET current_turn_participant_id = ? WHERE id = ?",
        )
        .bind(participant_id)
        .bind(session_id)
        .execute(&self.pool)
        .await
        {
            tracing::warn!(session_id, ?e, "recording the turn holder failed");
        }
    }

    /// Record how many laps of the ring this uninterrupted stretch has
    /// completed.
    ///
    /// `sessions.round_number` has existed since 0044 with **no writer at all** —
    /// `MAX(round_number)` was 0 across every session ever recorded. Exactly the
    /// shape [`set_current_turn`](Self::set_current_turn) found and closed for
    /// `current_turn_participant_id`, and closed the same way rather than by
    /// dropping the column: 0044 is applied and immutable, so removing it costs
    /// a new migration, and a lap count is the one number that says how far an
    /// unattended run actually got.
    ///
    /// **The stretch, not the session's lifetime.** It counts what
    /// `run_sequencer`'s `laps` counts, because it is written from it and from
    /// nowhere else — including the reset a user message performs. A column that
    /// tracked laps-ever and a cap that tracked laps-this-stretch would be two
    /// numbers with one name.
    ///
    /// Best-effort, like the turn holder: a failed write costs a UI hint and a
    /// post-hoc reading, never a turn.
    pub async fn set_round_number(&self, session_id: &str, round: u32) {
        if let Err(e) = sqlx::query("UPDATE sessions SET round_number = ? WHERE id = ?")
            .bind(round)
            .bind(session_id)
            .execute(&self.pool)
            .await
        {
            tracing::warn!(session_id, round, ?e, "recording the round number failed");
        }
    }

    /// Read back what [`set_round_number`](Self::set_round_number) wrote.
    ///
    /// Deliberately its own query rather than a field on [`Session`]: adding one
    /// there means `SESSION_COLUMNS`, every `query_as::<_, Session>` site and the
    /// generated TS bindings, which is a wider change than a lap count needs.
    /// A missing session reads as 0 — the column's own default, and the same
    /// answer as a session that has not completed a lap.
    pub async fn round_number(&self, session_id: &str) -> Result<u32> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT round_number FROM sessions WHERE id = ?")
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await
            .context("reading the round number")?;
        Ok(row.map(|(n,)| n.max(0) as u32).unwrap_or(0))
    }

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
            .filter(|p| p.enabled && p.participation_mode == MODE_ACTIVE)
            .collect();
        Ok(next_in_ring(&ring, current).cloned())
    }

    /// Point one participant at a different model. The external driver's
    /// per-slot model override lands here.
    pub async fn set_participant_model(
        &self,
        participant_id: i64,
        model_id: Option<&str>,
    ) -> Result<()> {
        sqlx::query("UPDATE session_participants SET model_id = ? WHERE id = ?")
            .bind(model_id)
            .bind(participant_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("setting the model of participant {participant_id}"))?;
        Ok(())
    }

    /// Set this participant's per-spawn effort / ultracode knobs (rc3 D12).
    /// `None` on either leaves that knob alone rather than clearing it — the
    /// caller passes only what it was given.
    pub async fn set_participant_spawn_knobs(
        &self,
        participant_id: i64,
        effort: Option<&str>,
        ultracode: Option<bool>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE session_participants \
             SET effort = COALESCE(?, effort), ultracode = COALESCE(?, ultracode) \
             WHERE id = ?",
        )
        .bind(effort)
        .bind(ultracode.map(i64::from))
        .bind(participant_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("setting the spawn knobs of participant {participant_id}"))?;
        Ok(())
    }

    /// Remember this participant's claude-code conversation id so the next
    /// spawn can `--resume` it instead of starting blank.
    ///
    /// rc3 **D10**: replaces `set_session_claude_id`, whose `match agent { "hands"
    /// => …, "eyes" => …, other => bail }` could only ever address two rows. A
    /// third participant's conversation was not stored somewhere else — it hit
    /// the `bail` arm and was dropped, so it restarted blank every respawn.
    pub async fn set_participant_claude_id(
        &self,
        participant_id: i64,
        claude_session_id: &str,
    ) -> Result<()> {
        sqlx::query("UPDATE session_participants SET claude_session_id = ? WHERE id = ?")
            .bind(claude_session_id)
            .bind(participant_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("recording claude session id for participant {participant_id}"))?;
        Ok(())
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
    /// The states that reach the empty-rotation case: an all-`on_mention`
    /// roster, a roster whose every active participant has been
    /// disabled (what disabling the last agent produces), and a session with no
    /// roster yet, since `ensure_session_roster` only runs pre-spawn.
    pub async fn all_active_voted_done(&self, session_id: &str) -> Result<bool> {
        let roster = self.participants_for_session(session_id).await?;
        Ok(roster
            .iter()
            .filter(|p| p.enabled && p.participation_mode == MODE_ACTIVE)
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
                 VALUES (?, ?, CASE WHEN ?3 IS NULL THEN ?4 ELSE NULL END, ?3)",
            )
            .bind(participant_id)
            .bind(message_id)
            .bind(*withheld_reason)
            // `now_utc()`, not `datetime('now')` (F3). SQLite's is zone-less
            // `YYYY-MM-DD HH:MM:SS` while everything else in this database is
            // RFC3339-Z — and this file already records, twice, what that
            // mismatch costs: a lexicographic compare between the two shapes
            // silently broke a guard once.
            .bind(crate::storage::now_utc())
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
            // Numbered, not positional: mixing `?` with `?N` renumbers the
            // trailing placeholders (the bare `?` after a `?3` becomes `?4`),
            // which is how the cursor's own id ended up unbound for one run.
            "UPDATE participant_cursors \
             SET last_read_message_id = MAX(last_read_message_id, ?1), \
                 updated_at = ?3 \
             WHERE participant_id = ?2",
        )
        .bind(high)
        .bind(participant_id)
        .bind(crate::storage::now_utc())
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
/// Who a delivered row is FROM, as the participant reading it sees.
///
/// **The wire carried no author at all until rc3 D23**, and three sessions'
/// worth of confusion trace back to that one gap. A participant handed four rows
/// received four anonymous strings: it could not tell the user's task from a
/// peer's aside from a host notice, so it inferred, and the inference showed up
/// as `s-81057bde`'s reviewer reporting "no task from the user and no HANDS
/// output" while the delivery table recorded eight rows handed to it. Both were
/// true. It had read them and could not tell what they were.
///
/// The slug rather than the display name (`ROLE · Model`), for two reasons: it
/// is ON the row, so labelling costs no lookup and cannot go stale mid-session;
/// and it is the same handle `@mention` parses, so a participant reading
/// `[eyes-2]` is reading the string the user would type to summon it. rc3 D20's
/// user-set label supersedes this when it lands — the label the peers read and
/// the label the user reads should be one string.
pub fn speaker_of(origin: &str, author: Option<&str>, label: Option<&str>) -> String {
    match origin {
        // The host's own injections and the user's typing are both "not a
        // peer", but they are not the same authority and must not read as one:
        // a system notice is bot-hq talking, and an agent that mistakes it for
        // the user has been handed a fabricated instruction.
        //
        // **Neither takes a label**, and that is not an omission: a label names
        // a PARTICIPANT, and there is no participant here to name. A host notice
        // that could be renamed is the D23 failure with a new coat of paint.
        "system" => "system".to_string(),
        "user" => "user".to_string(),
        // rc3 D20 (migration 0053): the user's name for this participant wins
        // over its slug, so the name its peers read and the name the user reads
        // are one string — which was the whole point of the label.
        //
        // Blank is not a name, exactly as in [`participant_display_name`]: a
        // whitespace label falls through to the slug rather than putting an
        // empty `[]` on the wire.
        //
        // A participant row whose author is missing predates the dual-write or
        // was written by a path that did not set it. Naming it `participant` is
        // less wrong than naming it nothing.
        _ => label
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .or(author)
            .unwrap_or("participant")
            .to_string(),
    }
}

/// What separates two rows inside ONE delivery — see
/// [`PersistedMessage::wire_batch`].
///
/// A blank line and nothing heavier. rc3 D23 already leads every row with its
/// `[speaker]`, so a blank line between them reads as a transcript and teaches
/// the participant no new format: the delimiter that matters is already there,
/// on the row, and a rule about `---` fences would be one more thing a prompt
/// has to explain and a body could counterfeit.
pub const WIRE_JOIN: &str = "\n\n";

/// The most bytes one row's BODY may put on a participant's stdin — see
/// [`PersistedMessage::wire`] for the incident this caps. ~50k tokens: far
/// above any legitimate message, far below what can wedge a context window.
/// `AppState`'s user-message cap matches it, so an accepted user message is
/// never truncated — this clamp catches what that gate cannot: rows already
/// on record, agent-authored dumps, and any future write path.
pub const WIRE_BODY_CLAMP_BYTES: usize = 200_000;

/// [`WIRE_BODY_CLAMP_BYTES`] applied: oversized bodies are cut at a char
/// boundary and carry a marker saying what was cut and where the rest lives.
/// The marker addresses the AGENT reading it — the actionable move (a file
/// read, or asking the user for a path) belongs in the text that replaced the
/// content, not in a doc nobody's subprocess ever sees.
fn clamped_body(body: &str) -> std::borrow::Cow<'_, str> {
    if body.len() <= WIRE_BODY_CLAMP_BYTES {
        return std::borrow::Cow::Borrowed(body);
    }
    let mut cut = WIRE_BODY_CLAMP_BYTES;
    while !body.is_char_boundary(cut) {
        cut -= 1;
    }
    std::borrow::Cow::Owned(format!(
        "{}\n\n[bot-hq: truncated on delivery — this message is {} bytes and the \
         per-message wire cap is {}. The full text is on the session record, not \
         in your context. Content this large belongs in a FILE read selectively \
         (grep/head), not in chat; if you need the remainder, ask for a path.]",
        &body[..cut],
        body.len(),
        WIRE_BODY_CLAMP_BYTES
    ))
}

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
    /// Who wrote it — the participant slug for `origin = "participant"`, and
    /// `"user"` for everything the host or the user authored. Read because the
    /// WIRE has to say it: see [`speaker_of`].
    pub author: Option<String>,
    /// The writing participant's user-set label (rc3 D20, migration 0053), or
    /// `None` when it has none — or when the row has no participant at all.
    ///
    /// **Resolved at READ time, from a join, not stored on the row.** Renaming a
    /// participant therefore re-labels what it already said, which is the right
    /// way round and matches `color`: the transcript shows who that participant
    /// IS, not a snapshot of what it was called that minute. Storing it would
    /// additionally freeze a name into every row, which is the mistake
    /// `render_wire` already avoids for the phase tag.
    pub speaker_label: Option<String>,
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

/// Qualified with `m.` because the channel read JOINs — see [`Storage::channel_page`].
const CHANNEL_COLUMNS: &str = "m.id, m.session_id, m.participant_id, m.origin, m.kind, \
     m.content, m.envelope, m.created_at, m.author, p.label AS speaker_label";

fn channel_from_row(r: &sqlx::sqlite::SqliteRow) -> ChannelMessage {
    use sqlx::Row;
    let envelope: Option<String> = r.get("envelope");
    ChannelMessage {
        id: r.get("id"),
        session_id: r.get("session_id"),
        participant_id: r.get("participant_id"),
        speaker_label: r.get("speaker_label"),
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
        author: r.get("author"),
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
    /// `Arc<str>` rather than `String` to match `PumpConfig::session_id` and the
    /// `MessagePersisted` / `BatchEmitter` threading this will flow into.
    session_id: Arc<str>,
    body: String,
    envelope: Option<Envelope>,
    /// Who wrote it, as the reader sees — see [`speaker_of`]. On the receipt
    /// rather than resolved at the write, because the write has no roster and
    /// the row already knows.
    speaker: String,
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
    ///
    /// **One answer also means one clamp.** A single row's body above
    /// [`WIRE_BODY_CLAMP_BYTES`] is truncated HERE, on the wire — the row in
    /// `messages` stays whole. s-f6a441ff: one 2,977,078-byte user paste (prod
    /// logs) rode a batch into both participants' subprocesses; once ingested,
    /// every subsequent prompt exceeded even the 1M window and the session
    /// died volleying "Prompt is too long" — unrecoverably, because the paste
    /// was lodged in each subprocess's own transcript where no later delivery
    /// decision can reach it. Clamping at the one place rows become stdin
    /// bytes means no single row can do that again, wherever it came from —
    /// a user paste, an agent's pasted dump, or a replayed backlog.
    pub fn wire(&self) -> String {
        // **The speaker leads.** Everything else in the wire decorates the
        // message; this says whose message it is, which is the one thing a
        // participant cannot work out for itself.
        format!(
            "[{}] {}",
            self.speaker,
            render_wire(self.envelope.as_ref(), &clamped_body(&self.body))
        )
    }

    /// The bytes a BATCH of rows puts on a participant's stdin, as ONE write.
    ///
    /// The turn path delivers a whole page at once rather than a row at a time
    /// — see [`ParticipantInput::deliver_batch`](crate::agents::ParticipantInput::deliver_batch)
    /// and the "one turn, one write" section of the sequencer's module doc. This
    /// is the same [`wire`](Self::wire) for each row, in the order given, with
    /// [`WIRE_JOIN`] between them; there is no batch-level decoration, because
    /// each row already carries the one thing that identifies it.
    ///
    /// **Order is the caller's and is load-bearing.** The backlog arrives in
    /// ascending id, so the newest row — typically the user's, since a user
    /// message is what wakes the ring — reads LAST. That is the whole point of
    /// the coalescing: a participant reads its peer's turn as context and the
    /// user's instruction as the thing it was just asked to do.
    pub fn wire_batch(msgs: &[Self]) -> String {
        msgs.iter().map(Self::wire).collect::<Vec<_>>().join(WIRE_JOIN)
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
            speaker: speaker_of(
            &row.origin,
            row.author.as_deref(),
            row.speaker_label.as_deref(),
        ),
        }
    }

    /// Who this row is from — the `[speaker]` the wire carries.
    pub fn speaker(&self) -> &str {
        &self.speaker
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
        // from `PumpConfig::session_id` — passes a refcount bump instead of
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
            // Resolved from the same two values the INSERT just wrote, so the
            // receipt says exactly what a re-read of the row would.
            // No label here, and it costs nothing: every caller that DELIVERS a
            // write-time receipt posts as `system` or `user`, neither of which
            // takes a label. Participant rows are written by the output pump
            // (`pump.rs`), which only notifies — its peers read the row back
            // through the ring, where the join supplies the label.
            speaker: speaker_of(origin, Some(legacy_author), None),
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
            // LEFT, not INNER: `origin = 'user'` and `origin = 'system'` rows
            // carry no participant by design (0044), and an inner join would
            // drop every user message and every host injection from every
            // backlog — the same NULL trap the exclusion clause below documents.
            "SELECT {CHANNEL_COLUMNS} FROM messages m \
             LEFT JOIN session_participants p ON p.id = m.participant_id \
             WHERE m.session_id = ?1 AND m.id > ?2 \
               AND (?3 IS NULL OR m.participant_id IS NULL OR m.participant_id <> ?3) \
               AND (?3 IS NULL OR m.kind NOT IN ('tool_use', 'tool_result', 'boot')) \
             ORDER BY m.id ASC LIMIT ?4"
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
    /// What a participant has not yet read — **prose and host notices only**.
    ///
    /// `tool_use` / `tool_result` / `boot` rows are excluded when reading FOR a
    /// participant (`exclude_participant` is `Some`), and included when reading
    /// for the UI (`None`), which is what renders the full transcript.
    ///
    /// `boot` is rc3 **D21**: orientation happens in parallel before the ring
    /// starts, and D21 is explicit that its output is *"persisted and shown to
    /// the USER, but not delivered to peers — three near-identical 'CL loaded'
    /// rows are exactly the noise the channel does not need"*. It rides this
    /// filter rather than getting a mechanism of its own, which is why D19a had
    /// to land first.
    ///
    /// The router forwarded a turn's buffered PROSE and nothing else. The ring
    /// drains every row past a cursor, and tool plumbing is rows — so without
    /// this filter each participant was handed every peer's raw tool JSON.
    /// Observed in `s-0d063183`: a participant was delivered
    /// `{"input":{"project":"cognotify"},"name":"…cl_index_search"}` and spent a
    /// turn correctly objecting that it was an envelope, not a message.
    ///
    /// It is not only noise. `tool_result` bodies are file reads, git output and
    /// CL dumps, so every participant was paying to read every peer's plumbing
    /// on every turn — the most plausible cause of the `Prompt is too long` that
    /// killed a participant on a 1M-token model.
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

    // ---- 0048: the seeded roles are the user's ---------------------------

    /// rc3's load-bearing distinction, as a query: after 0048, bot-hq claims to
    /// own no role at all.
    ///
    /// Asserted over EVERY row rather than over `hands`/`eyes` by name, because
    /// the claim in the reframe contract is about the product, not about two
    /// slugs — a later migration that seeds a third builtin role breaks the
    /// promise just as thoroughly, and should fail here.
    #[tokio::test]
    async fn no_role_is_flagged_builtin_after_0048() {
        let s = storage_with_0044().await;
        let flagged: Vec<String> =
            sqlx::query_scalar("SELECT slug FROM roles WHERE builtin <> 0")
                .fetch_all(s.pool())
                .await
                .unwrap();
        assert!(
            flagged.is_empty(),
            "bot-hq still claims to ship these roles: {flagged:?}"
        );
        // Non-vacuous: there ARE rows, so the query above had something to find.
        let total: i64 = sqlx::query_scalar("SELECT count(*) FROM roles")
            .fetch_one(s.pool())
            .await
            .unwrap();
        assert!(total >= 2, "no seeded roles at all — the assertion proves nothing");
    }

    /// The replacement for what `builtin` used to be asked, pinned end to end.
    ///
    /// `no_role_is_flagged_builtin_after_0048` above makes `builtin` permanently
    /// false, which silently broke the Roles tab's "clearing this box restores
    /// the built-in text" notice — it branched on that flag, so it started
    /// telling every user that clearing HANDS' prompt would leave HANDS with no
    /// instruction. The truth is the opposite: `read_system_prompt` falls back
    /// to `builtin_prose_for_role(<role slug>)`, which returns `HANDS_ROLE` in
    /// full.
    ///
    /// `builtin_prose_for_role` answers that honestly, but only if the role slug
    /// it keys on is the one a real roster's participants actually point at. So
    /// this walks a REAL roster rather than asserting the mapping against
    /// itself. rc3 D10 removed the role→AGENT hop the mapping used to make; what
    /// is left to check is that the seeded roles reach prose at all.
    #[tokio::test]
    async fn builtin_prose_for_role_matches_what_the_seeded_roster_falls_back_to() {
        use crate::agents::prompts::builtin_prose_for_role;

        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.ensure_session_roster("s1", MAX_SESSION_PARTICIPANTS).await.unwrap();
        let roster = s.participants_for_session("s1").await.unwrap();
        assert_eq!(roster.len(), 2, "the roster did not seed, so nothing is compared");

        for p in &roster {
            let role_id = p.role_id.expect("a seeded participant points at a role");
            let role = s.role_by_id(role_id).await.unwrap().expect("its role row exists");
            // Non-vacuous: an empty answer would make the Roles tab's notice
            // wrong in exactly the direction this test exists for.
            assert!(
                !builtin_prose_for_role(&role.slug).is_empty(),
                "role '{}' has no built-in prose, so clearing its prompt would leave \
                 participant '{}' with no identity at all",
                role.slug,
                p.slug
            );
            // And it is that ROLE's prose, not the other one's — a transposed
            // match arm reads as a model behaving strangely, not as a bug.
            assert!(
                builtin_prose_for_role(&role.slug).contains(&role.display_name),
                "role '{}' fell back to prose that does not name it",
                role.slug
            );
        }

        // A role the roster never seeds has nothing to fall back to — the other
        // arm of the notice, and the one that WAS right before.
        assert_eq!(builtin_prose_for_role("reviewer-2"), "");
    }

    /// 0044 seeded `hands` with `route_gated_command`, which is not a
    /// `Capability`. 0048 removes it.
    ///
    /// The second half is the one that matters: EVERY surviving slug on EVERY
    /// role must parse. Asserting only that the known-bad slug is gone would
    /// stay green if the seed grew a second unparseable one.
    #[tokio::test]
    async fn every_seeded_capability_slug_parses_after_0048() {
        let s = storage_with_0044().await;
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT slug, capabilities FROM roles")
                .fetch_all(s.pool())
                .await
                .unwrap();
        assert!(!rows.is_empty(), "no roles to check");

        let mut checked = 0usize;
        for (role, raw) in &rows {
            let slugs: Vec<String> = serde_json::from_str(raw)
                .unwrap_or_else(|e| panic!("role {role} capabilities is not a JSON array: {e}"));
            assert!(
                !slugs.contains(&"route_gated_command".to_string()),
                "role {role} still carries the stray `route_gated_command` grant"
            );
            assert!(
                !slugs.contains(&"declare_working".to_string()),
                "role {role} still grants the retired `declare_working` — \
                 migration 0057 scrubs it"
            );
            for slug in &slugs {
                assert!(
                    crate::agents::Capability::parse(slug).is_some(),
                    "role {role} grants `{slug}`, which Capability::parse drops on the floor"
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "no slugs were checked — the loop proves nothing");
    }

    /// The correction is a REMOVAL, not a rename — which only holds because the
    /// 0044 seed already carried the real slug. If it had not, dropping
    /// `route_gated_command` would have silently taken HANDS's gated-command
    /// grant away with it, and `every_seeded_capability_slug_parses_after_0048`
    /// would still be green.
    #[tokio::test]
    async fn hands_keeps_gated_bash_through_the_0048_correction() {
        let s = storage_with_0044().await;
        let hands = s.role_by_slug("hands").await.unwrap().expect("0044 seeds 'hands'");
        let caps = crate::agents::CapabilitySet::from_json(&hands.capabilities)
            .expect("hands capabilities must decode");
        assert!(
            caps.contains(crate::agents::Capability::GatedBash),
            "the stray slug's removal took `gated_bash` with it"
        );
        assert!(
            caps.contains(crate::agents::Capability::RunBash),
            "`gated_bash` without `run_bash` is the incoherent pair validate() refuses"
        );

        // **The parity claim for this migration, stated as an equality.** rc3 is
        // a reframe: 0048 corrects stored DATA and must change no behaviour. The
        // effective set is what any behaviour reads, and `Capability::parse`
        // already returned `None` for the stray slug, so `from_json` was already
        // dropping it — decoding the exact 0044 seed has to yield the same set
        // the corrected row does. If this ever differs, 0048 stopped being a
        // cleanup and started being a permission change.
        let seed_0044 = r#"["read_channel","post_channel","ask_user","park_approval",
          "route_gated_command","supersede_question","disposition_finding",
          "override_reviewer_block","halt","declare_working","run_terminal",
          "write_context_library","edit_files","run_bash","gated_bash",
          "close_session"]"#;
        let before = crate::agents::CapabilitySet::from_json(seed_0044)
            .expect("the 0044 seed must decode");
        assert_eq!(
            before, caps,
            "0048 changed HANDS's effective capabilities; it is only allowed to change the bytes"
        );
    }

    /// 0048's capability statement must be safe on a row the user has already
    /// edited: it rewrites the ONE stray element and leaves everything else,
    /// including grants that did not come from the seed, exactly as written.
    ///
    /// Re-runs the migration's own statement against a hand-edited row rather
    /// than re-running the whole migration, because sqlx will not re-apply an
    /// applied migration and the property under test is the statement's.
    #[tokio::test]
    async fn the_0048_capability_fix_does_not_clobber_a_hand_edited_list() {
        let s = storage_with_0044().await;
        // A user who has pruned HANDS down, added the stray back, and kept an
        // order of their own.
        let edited = r#"["close_session","route_gated_command","read_channel","post_channel","run_bash","gated_bash"]"#;
        sqlx::query("UPDATE roles SET capabilities = ? WHERE slug = 'hands'")
            .bind(edited)
            .execute(s.pool())
            .await
            .unwrap();

        let stmt = capability_fix_statement();
        sqlx::query(stmt).execute(s.pool()).await.unwrap();

        let after = s.role_by_slug("hands").await.unwrap().unwrap();
        let slugs: Vec<String> = serde_json::from_str(&after.capabilities).unwrap();
        // The stray element and only the stray element — the user's pruning,
        // their additions and their ORDER all survive untouched.
        assert_eq!(
            slugs,
            ["close_session", "read_channel", "post_channel", "run_bash", "gated_bash"],
            "the fix changed more than the one stray element"
        );

        // Idempotent: a second run is a no-op, not a further edit.
        sqlx::query(stmt).execute(s.pool()).await.unwrap();
        let again = s.role_by_slug("hands").await.unwrap().unwrap();
        assert_eq!(again.capabilities, after.capabilities, "the fix is not idempotent");
    }

    /// 0048's prose re-seed overwrites what 0046 wrote and NOTHING ELSE.
    ///
    /// The migration's header claims exactly that, and the claim is the reason
    /// it matches the old bytes rather than using 0046's `IS NULL` guard (which
    /// would match nothing, 0046 having just filled the column) or an
    /// unconditional `SET` (which would eat the user's prose). Both halves are
    /// asserted here because either one alone is satisfiable by a wrong
    /// statement: `IS NULL` passes the "edited row survives" half, and an
    /// unconditional SET passes the "0046's seed is replaced" half.
    #[tokio::test]
    async fn the_0048_prose_reseed_overwrites_0046_but_not_a_user_edit() {
        // Half 1 — a stock migrated database ends up on the NEW constant, so
        // 0048 really did overwrite the bytes 0046 had just written.
        let s = storage_with_0044().await;
        let eyes = s.role_by_slug("eyes").await.unwrap().unwrap();
        assert_eq!(
            eyes.description_prompt.as_deref(),
            Some(crate::agents::prompts::EYES_ROLE),
            "0048 did not overwrite 0046's seed"
        );

        // Half 2 — the statement, replayed against a row the user has edited,
        // leaves it alone. Replayed rather than re-migrated because sqlx will
        // not re-apply an applied migration.
        sqlx::query("UPDATE roles SET description_prompt = ? WHERE slug = 'eyes'")
            .bind("You are EYES. Be brief and be right.")
            .execute(s.pool())
            .await
            .unwrap();
        sqlx::query(prose_reseed_statement()).execute(s.pool()).await.unwrap();
        let after = s.role_by_slug("eyes").await.unwrap().unwrap();
        assert_eq!(
            after.description_prompt.as_deref(),
            Some("You are EYES. Be brief and be right."),
            "0048 clobbered a user-edited role prompt"
        );
    }

    /// The prose re-seed from `0048_roles_are_the_users.sql`, read out of the
    /// migration itself so the test cannot drift from what actually ran.
    fn prose_reseed_statement() -> &'static str {
        use std::sync::OnceLock;
        static STMT: OnceLock<String> = OnceLock::new();
        STMT.get_or_init(|| {
            let sql = include_str!("../../migrations/0048_roles_are_the_users.sql");
            let marker = "-- 3. Re-seed 'eyes' prose";
            let from = sql.find(marker).expect("0048 lost its section-3 marker");
            let start = sql[from..].find("UPDATE").expect("no UPDATE after the marker") + from;
            sql[start..].trim_end().to_string()
        })
    }

    /// The capability-correcting statement from `0048_roles_are_the_users.sql`,
    /// read out of the migration file itself so the test cannot drift from what
    /// actually ran. Parsed by its section marker rather than copied.
    fn capability_fix_statement() -> &'static str {
        use std::sync::OnceLock;
        static STMT: OnceLock<String> = OnceLock::new();
        STMT.get_or_init(|| {
            let sql = include_str!("../../migrations/0048_roles_are_the_users.sql");
            let marker = "-- 2. Drop the stray grant";
            let from = sql.find(marker).expect("0048 lost its section-2 marker");
            let start = sql[from..].find("UPDATE").expect("no UPDATE after the marker") + from;
            let end = sql[start..].find(";\n").expect("unterminated statement") + start + 1;
            sql[start..end].to_string()
        })
    }

    /// A prose-reseed UPDATE, read out of the migration itself so the test
    /// cannot drift from what actually ran.
    ///
    /// Each statement ends at `);\n` — the closing paren of its guard — which
    /// no line of the prose contains, unlike a bare `;`.
    fn reseed_statement(sql: &str, which: &str, section: &str) -> String {
        let from = sql
            .find(section)
            .unwrap_or_else(|| panic!("{which} lost its {section:?} marker"));
        let start = sql[from..].find("UPDATE").expect("no UPDATE after the marker") + from;
        let end = sql[start..].find(");\n").expect("unterminated statement") + start + 2;
        sql[start..end].to_string()
    }

    /// One of `0049_role_prose_drops_the_names.sql`'s two UPDATEs.
    fn drops_the_names_statement(section: &str) -> String {
        reseed_statement(
            include_str!("../../migrations/0049_role_prose_drops_the_names.sql"),
            "0049",
            section,
        )
    }

    /// `0050_close_learnings_ask_is_conditional.sql`'s single UPDATE (HANDS).
    fn close_learnings_ask_statement() -> String {
        reseed_statement(
            include_str!("../../migrations/0050_close_learnings_ask_is_conditional.sql"),
            "0050",
            "-- 1. HANDS",
        )
    }

    /// `0055_yield_discipline_stops_manufacturing_work.sql`'s single UPDATE.
    fn yield_discipline_statement() -> String {
        reseed_statement(
            include_str!("../../migrations/0055_yield_discipline_stops_manufacturing_work.sql"),
            "0055",
            "-- 1. HANDS",
        )
    }

    /// **The guard on migration 0055, both directions** — the same two-sided
    /// proof 0049 and 0050 carry: overwrite the previous seed, never a user's
    /// edit. The prose being moved is the yield discipline that taught HANDS
    /// to manufacture work instead of stopping (`s-86a81478`).
    #[tokio::test]
    async fn the_0055_prose_reseed_overwrites_0050s_seed_but_not_a_user_edit() {
        let s = storage_with_0044().await;

        // Half 1 — a stock migrated database carries the D35 discipline.
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        let prose = hands.description_prompt.expect("hands prose seeded");
        assert_eq!(
            prose,
            crate::agents::prompts::HANDS_ROLE,
            "0055 did not overwrite 0050's seed for hands"
        );
        // The point of the migration, spelled out: stopping is the move, and
        // a parked question stops nothing.
        assert!(prose.contains("Do not invent work to avoid stopping"));
        assert!(prose.contains("the session keeps working"));
        assert!(!prose.contains("never yield twice"), "the old trap is gone");

        // Half 2 — replayed against a row the user has edited, it leaves it be.
        let edit = "You are HANDS. Ship small, verified changes.";
        sqlx::query("UPDATE roles SET description_prompt = ? WHERE slug = 'hands'")
            .bind(edit)
            .execute(s.pool())
            .await
            .unwrap();
        sqlx::query(&yield_discipline_statement())
            .execute(s.pool())
            .await
            .unwrap();
        assert_eq!(
            s.role_by_slug("hands").await.unwrap().unwrap().description_prompt.as_deref(),
            Some(edit),
            "0055 clobbered a user-edited prompt"
        );
    }

    /// **The guard on migration 0049, both directions.**
    ///
    /// 0049 re-seeds both roles' prose because rc3 D10 took the agent names out
    /// of the constants, and the column has been the USER's to edit since 0046.
    /// A migration that overwrites a user's prose is a data-loss bug, and one
    /// that overwrites nothing leaves every install serving the old text while a
    /// fresh one serves the new — the divergence the byte-parity oracle exists
    /// to catch, arriving through the other door.
    ///
    /// Either half alone is satisfiable by a WRONG statement, which is why both
    /// are here: `WHERE description_prompt IS NULL` passes the edited-row half
    /// (it matches nothing after 0046) and an unconditional `SET` passes the
    /// overwrite half.
    #[tokio::test]
    async fn the_0049_prose_reseed_overwrites_the_previous_seed_but_not_a_user_edit() {
        // Half 1 — a stock migrated database ends up on the NEW constants, so
        // 0049 really did overwrite what 0046 and 0048 had written.
        let s = storage_with_0044().await;
        for (slug, expected) in [
            ("hands", crate::agents::prompts::HANDS_ROLE),
            ("eyes", crate::agents::prompts::EYES_ROLE),
        ] {
            let role = s.role_by_slug(slug).await.unwrap().unwrap();
            assert_eq!(
                role.description_prompt.as_deref(),
                Some(expected),
                "0049 did not overwrite the previous seed for {slug}"
            );
            // The point of the whole migration, spelled out: no agent name.
            let prose = role.description_prompt.unwrap();
            for banned in ["Brian", "Rain"] {
                assert!(!prose.contains(banned), "{slug} still names {banned}");
            }
        }

        // Half 2 — each statement, replayed against a row the user has edited,
        // leaves it alone. Replayed rather than re-migrated because sqlx will
        // not re-apply an applied migration.
        for (slug, section, edit) in [
            ("hands", "-- 1. HANDS", "You are HANDS. Ship small, verified changes."),
            ("eyes", "-- 2. EYES", "You are EYES. Be brief and be right."),
        ] {
            sqlx::query("UPDATE roles SET description_prompt = ? WHERE slug = ?")
                .bind(edit)
                .bind(slug)
                .execute(s.pool())
                .await
                .unwrap();
            sqlx::query(&drops_the_names_statement(section))
                .execute(s.pool())
                .await
                .unwrap();
            assert_eq!(
                s.role_by_slug(slug).await.unwrap().unwrap().description_prompt.as_deref(),
                Some(edit),
                "0049 clobbered a user-edited prompt for {slug}"
            );
        }
    }

    /// **The guard on migration 0050, both directions** — the same instrument as
    /// the 0049 test above, for the reseed rc3 D15 needed.
    ///
    /// D15 made HANDS' close-out learnings ask conditional ("writing nothing is
    /// the expected outcome"), which changes `HANDS_ROLE`, which 0049 had
    /// already seeded byte-for-byte into the database. 0049 is applied and
    /// immutable, so the only correct move is this new migration — and it needs
    /// the same two-sided proof: it must overwrite 0049's seed, and it must
    /// leave a row the user has edited alone.
    #[tokio::test]
    async fn the_0050_prose_reseed_overwrites_0049s_seed_but_not_a_user_edit() {
        let s = storage_with_0044().await;

        // Half 1 — a stock migrated database is on the CONDITIONAL wording.
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        let prose = hands.description_prompt.expect("hands prose seeded");
        assert_eq!(
            prose,
            crate::agents::prompts::HANDS_ROLE,
            "0050 did not overwrite 0049's seed for hands"
        );
        // The point of the migration, spelled out: the ask is conditional and
        // silence is blameless (D15).
        assert!(prose.contains("only if this session turned up something"));
        assert!(prose.contains("Writing nothing is the expected outcome"));

        // Half 2 — replayed against a row the user has edited, it leaves it be.
        let edit = "You are HANDS. Ship small, verified changes.";
        sqlx::query("UPDATE roles SET description_prompt = ? WHERE slug = 'hands'")
            .bind(edit)
            .execute(s.pool())
            .await
            .unwrap();
        sqlx::query(&close_learnings_ask_statement())
            .execute(s.pool())
            .await
            .unwrap();
        assert_eq!(
            s.role_by_slug("hands").await.unwrap().unwrap().description_prompt.as_deref(),
            Some(edit),
            "0050 clobbered a user-edited prompt"
        );

        // And EYES is untouched by 0050 — the ask was never addressed to a role
        // with no `write_context_library` grant.
        assert_eq!(
            s.role_by_slug("eyes").await.unwrap().unwrap().description_prompt.as_deref(),
            Some(crate::agents::prompts::EYES_ROLE),
        );
    }

    // ---- 0046: role prose lives in the database --------------------------

    /// **The drift oracle for migration 0046.**
    ///
    /// 0046 seeds `roles.description_prompt` with the verbatim bytes of
    /// `HANDS_ROLE` / `EYES_ROLE`. Two copies of ~23KB of prose now exist — the
    /// SQL literal and the Rust constant — and nothing in the compiler relates
    /// them. Without this test, editing `prompts.rs` would leave every existing
    /// install serving the OLD prose from its database (0046 already applied,
    /// migrations never re-run) while a fresh install serves the new text, and
    /// the suite would stay green through the whole divergence.
    ///
    /// This opens a database that has actually run the migration and compares
    /// the stored bytes to the constants, so it also covers the SQL escaping —
    /// a mis-escaped quote is a byte difference like any other.
    ///
    /// **If this fails, do not delete it and do not edit the migration.** 0046
    /// is applied and therefore immutable; changing a byte of it breaks boot.
    /// The fix is a NEW migration that re-seeds, plus updating the constants.
    #[tokio::test]
    async fn seeded_role_prose_is_byte_identical_to_the_hardcoded_constants() {
        let s = storage_with_0044().await;

        let hands = s.role_by_slug("hands").await.unwrap().expect("0044 seeds 'hands'");
        assert_eq!(
            hands.description_prompt.as_deref(),
            Some(crate::agents::prompts::HANDS_ROLE),
            "roles.description_prompt for 'hands' has drifted from HANDS_ROLE"
        );

        let eyes = s.role_by_slug("eyes").await.unwrap().expect("0044 seeds 'eyes'");
        assert_eq!(
            eyes.description_prompt.as_deref(),
            Some(crate::agents::prompts::EYES_ROLE),
            "roles.description_prompt for 'eyes' has drifted from EYES_ROLE"
        );

        // Guard against the assertion above passing vacuously. `assert_eq!` on
        // two `None`s would be green, and a migration that seeded nothing is
        // exactly the failure this whole test exists to catch.
        assert!(
            !crate::agents::prompts::HANDS_ROLE.is_empty()
                && !crate::agents::prompts::EYES_ROLE.is_empty(),
            "the constants are empty — the comparison above proves nothing"
        );
    }

    /// The spawn path resolves prose through `role_id`, not through a role slug,
    /// so `role_by_id` has to agree with `role_by_slug` about which row is which.
    /// A transposed lookup would hand HANDS's prompt to EYES — a swap that reads
    /// as a model behaving strangely, not as a database bug.
    #[tokio::test]
    async fn role_by_id_returns_the_same_row_as_role_by_slug() {
        let s = storage_with_0044().await;
        for slug in ["hands", "eyes"] {
            let by_slug = s.role_by_slug(slug).await.unwrap().unwrap();
            let by_id = s.role_by_id(by_slug.id).await.unwrap().unwrap();
            assert_eq!(by_id, by_slug, "role_by_id disagreed with role_by_slug");
        }
        // An id no role has is `None`, not an error and not row 1 — the spawn
        // path treats a stale `role_id` as "no prose" and needs that distinction.
        let missing = s.role_by_id(9_999).await.unwrap();
        assert!(missing.is_none(), "an unknown role id must resolve to None");
    }

    /// An edit to the row is the entire point of 0046: the user could not touch
    /// the prose while it lived in the binary. This proves the column is a
    /// writable source, not a decorative copy — a read-only seed would satisfy
    /// the oracle above and still leave the feature unbuilt.
    #[tokio::test]
    async fn an_edited_role_row_is_what_a_later_read_returns() {
        let s = storage_with_0044().await;
        sqlx::query("UPDATE roles SET description_prompt = ? WHERE slug = 'hands'")
            .bind("You are HANDS. Ship small, verified changes.")
            .execute(s.pool())
            .await
            .unwrap();

        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        assert_eq!(
            hands.description_prompt.as_deref(),
            Some("You are HANDS. Ship small, verified changes.")
        );
        // The other role is untouched — an edit is per-row, not per-table.
        let eyes = s.role_by_slug("eyes").await.unwrap().unwrap();
        assert_eq!(
            eyes.description_prompt.as_deref(),
            Some(crate::agents::prompts::EYES_ROLE)
        );
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
        // 0044 seeded this `builtin = 1`; 0048 flips it, because these are the
        // user's two roles and bot-hq ships none. Pinned per-row here, and over
        // the whole table by `no_role_is_flagged_builtin_after_0048`.
        assert!(!hands.builtin, "bot-hq still claims to own HANDS");
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
        // `CapabilitySet::from_slugs`, which is a LEGAL configuration — so
        // accepting them would make a malformed write indistinguishable from a
        // deliberate one.
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
        d.participation_mode = "on_mention".into();
        let created = s.create_role(&d).await.unwrap();

        assert_eq!(created.slug, "code-reviewer");
        assert_eq!(created.display_name, "Code Reviewer");
        assert_eq!(created.description_prompt.as_deref(), Some("be terse"));
        // D8: the Roles tab owns the default model, so this column has to
        // round-trip or the tab's model select is a control that does nothing.
        assert_eq!(created.default_model_id.as_deref(), Some("m1"));
        assert_eq!(created.participation_mode, "on_mention");
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
        // Asserted on fields a collision would actually have clobbered;
        // `builtin` is 0 on every row since 0048 and so proves nothing here.
        let seeded = s.role_by_slug("hands").await.unwrap().unwrap();
        assert_eq!(seeded.display_name, "HANDS");
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
        s.ensure_session_roster("s1", MAX_SESSION_PARTICIPANTS).await.unwrap();
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
            .insert_participant("s1", "hands", "Brian", Some(hands.id), None,
                                &hands.capabilities, "active", 0)
            .await
            .unwrap();
        s.set_role_archived(hands.id, true).await.unwrap();

        let mut d = draft("Executor");
        d.capabilities = r#"[ "read_channel" ]"#.into();
        d.participation_mode = "on_mention".into();
        let updated = s.update_role(hands.id, &d).await.unwrap();

        // The edit lands, normalised the same way a create is — an update that
        // stored the field's raw text would leave the column holding a second
        // spelling of the same set.
        assert_eq!(updated.capabilities, r#"["read_channel"]"#);
        // And the mode is the caller's, not a default. Pinned here because
        // demoting a role to `on_mention` is how the ring stops scheduling it:
        // an update that ignored this would leave the role looking demoted in
        // the tab while its participants kept taking turns.
        assert_eq!(updated.participation_mode, "on_mention");

        // `update_role` does not touch `builtin` at all — the flag records
        // provenance and a save is not a provenance change. Since 0048 leaves
        // every seeded row at 0, asserting `!updated.builtin` would pass on an
        // UPDATE that wrongly zeroed it; so set it first and prove the save
        // leaves the 1 alone.
        sqlx::query("UPDATE roles SET builtin = 1 WHERE id = ?")
            .bind(hands.id)
            .execute(s.pool())
            .await
            .unwrap();
        let resaved = s.update_role(hands.id, &d).await.unwrap();
        assert!(resaved.builtin, "update_role overwrote `builtin`");
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

    /// The row-count check in [`Storage::update_role`] is what DIAGNOSES a
    /// missing id — and it is the only thing that does.
    ///
    /// The test above asserts `update_role(9999, ..).is_err()`, and that
    /// assertion is green with the `changed == 0` guard deleted: the function
    /// re-reads the row it just wrote, `role_by_id(9999)` answers `None`, and
    /// the `with_context` on that `Option` produces an error of its own. So
    /// err-vs-ok cannot tell the two builds apart. Verified by running the whole
    /// suite with the guard neutered — 1148 lib tests passed.
    ///
    /// What the guard buys is the difference between the two messages, and that
    /// difference is not cosmetic. "vanished between update and read" describes
    /// a row that existed for the UPDATE and was gone microseconds later — a
    /// should-be-impossible race, and a full afternoon of hunting for a
    /// concurrency bug. The truth is the flat, ordinary case of an id that never
    /// named anything: a stale roles-tab row, a caller's uninitialised `0`. The
    /// guard is what says so.
    #[tokio::test]
    async fn updating_a_missing_role_blames_the_id_not_an_impossible_race() {
        let s = storage_with_0044().await;
        let err = s
            .update_role(9999, &draft("Ghost"))
            .await
            .expect_err("a write to an id nothing holds must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("9999") && msg.contains("does not exist"),
            "expected the missing id to be named, got {msg:?}"
        );
        assert!(
            !msg.contains("vanished"),
            "a plain bad id was reported as a lost-row race, which sends the \
             reader hunting a concurrency bug that did not happen: {msg:?}"
        );
    }

    /// **A participant reads its peers' PROSE, not their plumbing.**
    ///
    /// The router forwarded a turn's buffered prose; the ring drains rows, and
    /// tool calls are rows. Without the kind filter every participant was handed
    /// every peer's raw `tool_use` / `tool_result` JSON — noise it cannot act on,
    /// and `tool_result` bodies are file reads and CL dumps, so it was also the
    /// bulk of what filled their context windows.
    ///
    /// The UI read (`exclude_participant = None`) must still see everything, or
    /// the transcript stops showing what the agents actually did.
    #[tokio::test]
    async fn a_participant_is_delivered_prose_but_never_a_peers_tool_plumbing() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "t", None).await.unwrap();
        s.ensure_session_roster("s1", MAX_SESSION_PARTICIPANTS).await.unwrap();
        let roster = s.participants_for_session("s1").await.unwrap();
        let (me, peer) = (roster[0].id, roster[1].id);

        for (kind, body) in [
            (MessageKind::Text, "real prose"),
            (MessageKind::ToolUse, r#"{"name":"cl_index_search"}"#),
            (MessageKind::ToolResult, r#"{"content":"a huge file dump"}"#),
            // rc3 D21: what a peer said while ORIENTING. The user sees it; a
            // peer learns nothing from "CL loaded" three times over.
            (MessageKind::Boot, "CL loaded for bot-hq"),
        ] {
            s.post_to_channel("s1", "participant", Some(&roster[1].slug), kind.as_str(), body, None)
                .await
                .unwrap();
        }
        s.post_to_channel("s1", "system", None, MessageKind::SystemNotice.as_str(), "host note", None)
            .await
            .unwrap();

        let mine: Vec<String> = s
            .unread_for_participant(me)
            .await
            .unwrap()
            .rows
            .into_iter()
            .map(|r| r.content)
            .collect();
        assert!(mine.iter().any(|c| c == "real prose"), "prose must arrive");
        assert!(mine.iter().any(|c| c == "host note"), "host notices must arrive");
        assert!(
            !mine.iter().any(|c| c.contains("cl_index_search")),
            "a peer's tool CALL must not be delivered: {mine:?}"
        );
        assert!(
            !mine.iter().any(|c| c.contains("a huge file dump")),
            "a peer's tool RESULT must not be delivered — this is what filled context windows: {mine:?}"
        );
        assert!(
            !mine.iter().any(|c| c.contains("CL loaded")),
            "a peer's BOOT output must not be delivered (rc3 D21): {mine:?}"
        );

        // The UI read is unfiltered, or the transcript loses what agents did.
        let all = s.channel_after("s1", 0, 100).await.unwrap();
        assert_eq!(all.rows.len(), 5, "the UI still sees every row, boot included");
        let _ = peer;
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
        s.insert_participant("s1", "hands", "Brian", Some(hands.id), None,
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
            .insert_participant("s1", "hands", "Brian", Some(hands.id), None,
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
            color: None,
            label: None,
            effort: None,
            ultracode: None,
            claude_session_id: None,
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
        // `on_mention`) before it completes. `current` is then not IN the ring, so
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

    /// rc3 **D20**: two participants of one role do not render identically.
    #[test]
    fn a_second_participant_of_one_role_carries_its_ordinal() {
        // The reported case, exactly: two reviewers, one role, one model.
        assert_eq!(
            participant_display_name(Some("EYES"), Some("DeepSeek V4 Pro"), "eyes", None),
            "EYES · DeepSeek V4 Pro"
        );
        assert_eq!(
            participant_display_name(Some("EYES"), Some("DeepSeek V4 Pro"), "eyes-2", None),
            "EYES-2 · DeepSeek V4 Pro",
            "the second reviewer must not read the same as the first"
        );
        // The first of a role takes no suffix, so a one-reviewer session is
        // unchanged — which is the common case and must stay quiet.
        assert_eq!(participant_display_name(Some("HANDS"), None, "hands", None), "HANDS");
        assert_eq!(participant_display_name(Some("HANDS"), None, "hands-3", None), "HANDS-3");
    }

    /// rc3 **D20**'s other half (migration 0053): the user names a participant,
    /// and that name wins over the ordinal.
    #[test]
    fn a_user_set_label_replaces_the_role_and_its_ordinal() {
        // The reported case again, but named rather than numbered: `EYES-2` was
        // an improvement on two identical bylines, and it still says nothing
        // about which reviewer this is. A label does.
        assert_eq!(
            participant_display_name(
                Some("EYES"),
                Some("DeepSeek V4 Pro"),
                "eyes-2",
                Some("Skeptic")
            ),
            "Skeptic · DeepSeek V4 Pro",
            "the label replaces the role AND its ordinal, and nothing else"
        );
        // **The model suffix survives.** What a participant runs is a different
        // fact from what the user called it, and D8's per-participant model
        // picker exists precisely so that fact is visible.
        assert_eq!(
            participant_display_name(None, Some("Claude Opus 5"), "hands", Some("Driver")),
            "Driver · Claude Opus 5"
        );
        assert_eq!(
            participant_display_name(Some("HANDS"), None, "hands", Some("Driver")),
            "Driver"
        );
    }

    #[test]
    fn a_blank_label_is_not_a_name() {
        // Empty and whitespace both fall back to today's rendering rather than
        // leaving an empty byline — the same `clean` every other field on this
        // path goes through. A UI that writes `""` for an untouched input must
        // not thereby erase the participant's name.
        for blank in ["", "   ", "\t", "\n "] {
            assert_eq!(
                participant_display_name(
                    Some("EYES"),
                    Some("DeepSeek V4 Pro"),
                    "eyes-2",
                    Some(blank)
                ),
                "EYES-2 · DeepSeek V4 Pro",
                "{blank:?} is not a name"
            );
        }
        // And the label is trimmed rather than rendered with its padding.
        assert_eq!(
            participant_display_name(Some("EYES"), None, "eyes-2", Some("  Skeptic  ")),
            "Skeptic"
        );
    }

    /// rc3 **D20** (migration 0053): `@` resolves the user's label as well as
    /// the slug.
    ///
    /// Without this the label would break the property `speaker_of`'s own doc
    /// rests on — *"a participant reading `[eyes-2]` is reading the string the
    /// user would type to summon it"*. Putting a label on the wire and leaving
    /// mentions slug-only would show peers a name that summons nobody.
    #[tokio::test]
    async fn a_mention_resolves_a_label_as_well_as_a_slug() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "t", None).await.unwrap();
        // `false` = the whole roster: the first-role-only variant has no `eyes`.
        s.ensure_session_roster("s1", MAX_SESSION_PARTICIPANTS).await.unwrap();
        let eyes = s.participant_by_slug("s1", "eyes").await.unwrap().unwrap();
        sqlx::query("UPDATE session_participants SET label = ? WHERE id = ?")
            .bind("  Skeptic  ")
            .bind(eyes.id)
            .execute(&s.pool)
            .await
            .unwrap();

        // The slug still resolves — a label is an alias, never a replacement.
        assert_eq!(
            s.participant_by_mention("s1", "eyes").await.unwrap().map(|p| p.id),
            Some(eyes.id)
        );
        // And so does the label, case-folded and trimmed the same way the
        // mention parser normalises the token it hands over.
        for token in ["skeptic", "Skeptic", "SKEPTIC"] {
            assert_eq!(
                s.participant_by_mention("s1", token).await.unwrap().map(|p| p.id),
                Some(eyes.id),
                "@{token} names the participant the user called Skeptic"
            );
        }
        // A token that is neither is nobody — ordinary prose (D1), never an
        // error.
        assert!(s.participant_by_mention("s1", "nobody").await.unwrap().is_none());
    }

    /// A slug OUTRANKS a label, and the collision is the reason.
    #[tokio::test]
    async fn a_label_cannot_shadow_another_participants_slug() {
        // The user names one participant after another's slug. The slug is the
        // key — assigned at invite, unique by constraint, unchangeable — so it
        // has to win: otherwise renaming one participant silently redirects
        // every summons meant for the other.
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "t", None).await.unwrap();
        s.ensure_session_roster("s1", MAX_SESSION_PARTICIPANTS).await.unwrap();
        let hands = s.participant_by_slug("s1", "hands").await.unwrap().unwrap();
        let eyes = s.participant_by_slug("s1", "eyes").await.unwrap().unwrap();
        sqlx::query("UPDATE session_participants SET label = ? WHERE id = ?")
            .bind("hands")
            .bind(eyes.id)
            .execute(&s.pool)
            .await
            .unwrap();

        assert_eq!(
            s.participant_by_mention("s1", "hands").await.unwrap().map(|p| p.id),
            Some(hands.id),
            "@hands is the participant whose SLUG is hands, whatever anyone was renamed to"
        );
    }

    #[test]
    fn an_ordinal_is_only_a_trailing_number_that_a_duplicate_would_have() {
        assert_eq!(slug_ordinal("eyes-2"), Some(2));
        assert_eq!(slug_ordinal("eyes-10"), Some(10));
        // `-1` is not a suffix `first_free_slug` ever assigns: the first of a
        // role is the bare slug, and suffixes start at 2.
        assert_eq!(slug_ordinal("eyes-1"), None);
        assert_eq!(slug_ordinal("eyes"), None);
        assert_eq!(slug_ordinal("code-reviewer"), None);
        assert_eq!(slug_ordinal("-2"), None, "a suffix needs something to suffix");
        assert_eq!(slug_ordinal(""), None);
        // A role NAMED with a trailing number keeps it, which is the same string
        // either way — the worst case is a suffix that was already there.
        assert_eq!(slug_ordinal("agent-7"), Some(7));
    }

    /// The display rule and its TypeScript twin render the same participant on
    /// different surfaces, so the ordinal has to be in both. This is the Rust
    /// half; `participants.test.ts` holds the other.
    #[test]
    fn the_ordinal_survives_the_model_being_gone() {
        assert_eq!(participant_display_name(Some("EYES"), None, "eyes-2", None), "EYES-2");
        // No ROLE, though, means there is nothing to number — the model alone is
        // not a role and two of them are not "the second EYES".
        assert_eq!(
            participant_display_name(None, Some("DeepSeek V4 Pro"), "eyes-2", None),
            "DeepSeek V4 Pro"
        );
        assert_eq!(participant_display_name(None, None, "eyes-2", None), "eyes-2");
    }

    #[tokio::test]
    async fn the_ring_skips_the_summonable_and_wraps() {
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.insert_participant("s1", "a", "A", None, None, "[]", "active", 0).await.unwrap();
        s.insert_participant("s1", "adv", "Adv", None, None, "[]", "on_mention", 1).await.unwrap();
        s.insert_participant("s1", "c", "C", None, None, "[]", "active", 2).await.unwrap();

        // A user message resets to the first active participant.
        let first = s.next_active_participant("s1", None).await.unwrap().unwrap();
        assert_eq!(first.slug, "a");
        // The `on_mention` participant is SKIPPED, not given a no-op turn. The
        // only thing that reaches it is a summons (rc3 D17), and a summons does
        // not come through here.
        let second = s.next_active_participant("s1", Some(&first)).await.unwrap().unwrap();
        assert_eq!(second.slug, "c", "an on_mention participant must not take a ring turn");
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

        // What it does NOT prevent, and must not: an `on_mention` participant at
        // the same position, because it never takes a RING turn and its position
        // is retained purely against a later promotion.
        s.insert_participant("s1", "adv", "Adv", None, None, "[]", "on_mention", 0)
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
    async fn consensus_needs_every_active_participant_and_ignores_the_summonable() {
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let a = s.insert_participant("s1", "a", "A", None, None, "[]", "active", 0).await.unwrap();
        let c = s.insert_participant("s1", "c", "C", None, None, "[]", "active", 1).await.unwrap();
        s.insert_participant("s1", "adv", "O", None, None, "[]", "on_mention", 2).await.unwrap();

        assert!(!s.all_active_voted_done("s1").await.unwrap());
        s.set_done_vote(a, true).await.unwrap();
        assert!(!s.all_active_voted_done("s1").await.unwrap(), "one vote is not consensus");
        s.set_done_vote(c, true).await.unwrap();
        assert!(
            s.all_active_voted_done("s1").await.unwrap(),
            "an on_mention participant must not be required to vote — 1 active \
             + 3 summonable would otherwise need 4 yields to halt, and three of \
             them are never handed a turn to yield from"
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
        s.ensure_session_roster("s1", MAX_SESSION_PARTICIPANTS).await.unwrap();
        let brian = s.participant_by_slug("s1", "hands").await.unwrap().unwrap().id;
        let rain = s.participant_by_slug("s1", "eyes").await.unwrap().unwrap().id;

        let m1 = s.post_to_channel("s1", "user", None, "text", "one", None).await.unwrap();
        let m2 = s
            .post_to_channel("s1", "participant", Some("hands"), "text", "two", None)
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

    /// **Delivery timestamps are RFC3339-Z like every other time in this
    /// database** (F3).
    ///
    /// They were written with SQLite's own `datetime('now')`, which is zone-less
    /// `YYYY-MM-DD HH:MM:SS`. This file explains twice, at length, why that
    /// matters: the two shapes cannot be compared lexicographically, and a guard
    /// that tried it broke silently once. Nothing reads these columns TODAY,
    /// which is exactly why this is worth pinning now — the first reader would
    /// inherit the bug rather than introduce it.
    #[tokio::test]
    async fn a_delivery_timestamps_itself_in_the_shape_everything_else_uses() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "t", None).await.unwrap();
        let p = s
            .insert_participant("s1", "hands", "HANDS", None, None, "[]", "active", 0)
            .await
            .unwrap();
        let m = s
            .post_to_channel("s1", "user", None, "text", "go", None)
            .await
            .unwrap();
        s.commit_delivery(p, &[(m.message_id(), None)]).await.unwrap();

        let stamps: Vec<String> = sqlx::query_scalar(
            "SELECT delivered_at FROM participant_deliveries WHERE participant_id = ?",
        )
        .bind(p)
        .fetch_all(s.pool())
        .await
        .unwrap();
        assert_eq!(stamps.len(), 1);
        assert!(
            stamps[0].ends_with('Z') && stamps[0].contains('T'),
            "delivered_at is not RFC3339-Z: {:?} — a zone-less stamp cannot be \
             compared with `now_utc()` output, and this file has paid for that \
             mistake before",
            stamps[0]
        );
        let cursor: String = sqlx::query_scalar(
            "SELECT updated_at FROM participant_cursors WHERE participant_id = ?",
        )
        .bind(p)
        .fetch_one(s.pool())
        .await
        .unwrap();
        assert!(
            cursor.ends_with('Z') && cursor.contains('T'),
            "the cursor's updated_at is not RFC3339-Z: {cursor:?}"
        );
    }

    /// **A cursor is RFC3339-Z from BIRTH, not from its first delivery**
    /// (round-2 audit R5).
    ///
    /// The guard above already named `participant_cursors.updated_at` and
    /// already asserted RFC3339-Z on it — and 85 of 90 rows in the live
    /// database were zone-less anyway. It reached the column through the
    /// UPDATE path (`commit_delivery` binds `now_utc()`), so it only ever
    /// inspected a row that had already been corrected. The INSERT omitted
    /// `updated_at` entirely and let the column's `datetime('now')` DEFAULT
    /// fire, which is what every cursor was born with.
    ///
    /// So this asserts on a freshly inserted participant with NO delivery
    /// behind it. That is the whole difference between the two tests, and it is
    /// the fixture-shape rule again: a fixture that reaches the value through
    /// the path that writes it correctly cannot see the path that does not.
    #[tokio::test]
    async fn a_cursor_is_rfc3339_before_any_delivery_touches_it() {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "t", None).await.unwrap();

        // Both seeding paths: the single insert, and the roster transaction.
        let solo = s
            .insert_participant("s1", "hands", "HANDS", None, None, "[]", "active", 0)
            .await
            .unwrap();
        s.create_session("s2", "t", None).await.unwrap();
        s.ensure_session_roster("s2", 2).await.unwrap();
        let seeded: Vec<i64> = s
            .participants_for_session("s2")
            .await
            .unwrap()
            .iter()
            .map(|p| p.id)
            .collect();

        for pid in std::iter::once(solo).chain(seeded) {
            let stamp: String = sqlx::query_scalar(
                "SELECT updated_at FROM participant_cursors WHERE participant_id = ?",
            )
            .bind(pid)
            .fetch_one(s.pool())
            .await
            .unwrap();
            assert!(
                stamp.contains('T') && stamp.ends_with('Z'),
                "cursor {pid} was born zone-less: {stamp:?} — the column's \
                 `datetime('now')` DEFAULT fired because the INSERT omitted it, \
                 and ' ' (0x20) sorts before 'T' (0x54), so this row reads as \
                 EARLIER than midnight of its own day in every lexicographic \
                 window over it"
            );
        }
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
            .post_to_channel("s1", "participant", Some("hands"), MessageKind::Text.as_str(), "hello", None)
            .await
            .unwrap()
            .message_id();
        assert!(id > 0, "legacy insert_message must still work post-0044");
        s.insert_message("s1", MessageKind::Text, "hi back")
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
        assert_eq!(msgs[0].author, "hands");
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
            .insert_participant("s1", "hands", "Brian", Some(hands.id), None,
                                &hands.capabilities, "active", 0)
            .await
            .unwrap();
        let r = s
            .insert_participant("s1", "eyes", "Rain", Some(eyes.id), None,
                                &eyes.capabilities, "active", 1)
            .await
            .unwrap();

        let m1 = s.post_to_channel("s1", "user", None, "text", "do the thing", None)
            .await.unwrap();
        let m2 = s.post_to_channel("s1", "participant", Some("hands"), "text", "done",
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
        s.ensure_session_roster("s1", MAX_SESSION_PARTICIPANTS).await.unwrap();
        let rain = s.participant_by_slug("s1", "eyes").await.unwrap().unwrap().id;
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
        s.ensure_session_roster("s1", MAX_SESSION_PARTICIPANTS).await.unwrap();
        let brian = s.participant_by_slug("s1", "hands").await.unwrap().unwrap().id;
        let rain = s.participant_by_slug("s1", "eyes").await.unwrap().unwrap().id;

        let by_brian = s
            .post_to_channel("s1", "participant", Some("hands"), "text", "my turn", None)
            .await
            .unwrap();
        let by_rain = s
            .post_to_channel("s1", "participant", Some("eyes"), "text", "review", None)
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
        let eyes_unread: Vec<i64> = s
            .unread_for_participant(rain)
            .await
            .unwrap()
            .rows
            .iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(
            eyes_unread,
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
        s.ensure_session_roster("s1", MAX_SESSION_PARTICIPANTS).await.unwrap();
        s.ensure_session_roster("s2", MAX_SESSION_PARTICIPANTS).await.unwrap();
        let brian1 = s.participant_by_slug("s1", "hands").await.unwrap().unwrap().id;
        let brian2 = s.participant_by_slug("s2", "hands").await.unwrap().unwrap().id;
        assert_ne!(brian1, brian2, "precondition: two rosters, two 'brian' rows");

        let known = s
            .post_to_channel("s1", "participant", Some("hands"), "text", "mine", None)
            .await
            .unwrap();
        let unknown = s
            .post_to_channel("s1", "participant", Some("nobody"), "text", "orphan", None)
            .await
            .unwrap();
        let system = s
            .post_to_channel("s1", "system", Some("hands"), "system_notice", "notice", None)
            .await
            .unwrap();
        let slugless = s
            .post_to_channel("s1", "participant", None, "text", "anonymous", None)
            .await
            .unwrap();
        let elsewhere = s
            .post_to_channel("s2", "participant", Some("hands"), "text", "hers", None)
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
            .insert_participant("s1", "hands", "Brian", Some(hands.id), None,
                                &hands.capabilities, "active", 0)
            .await
            .unwrap();

        // The production write path: `post_to_channel` with the participant's
        // own slug. rc3 D10 makes that slug role-derived, so it is `hands` and
        // no longer an agent name.
        s.post_to_channel("s1", "participant", Some("hands"), "text", "work", None)
            .await
            .unwrap();
        s.insert_message("s1", MessageKind::Text, "reply").await.unwrap();

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
            .post_to_channel("s1", "participant", Some("eyes"), MessageKind::Text.as_str(), "no roster yet", None)
            .await
            .unwrap()
            .message_id();
        assert!(id > 0, "logging must never depend on the roster existing");
        let rows = all_rows(&s, "s1").await;
        assert_eq!(rows[0].participant_id, None);
        assert_eq!(rows[0].origin, "participant");
        // The legacy path still attributes it correctly.
        let legacy = s.messages_for_session("s1", None).await.unwrap();
        assert_eq!(legacy[0].author, "eyes");
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

    /// **The default roster is derived from the ROLES, not from two literal
    /// slugs** (rc3 D10), and it still produces exactly today's session.
    #[tokio::test]
    async fn the_default_roster_is_role_derived_in_creation_order() {
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        // Each role's own default model — the source spawn reads now, in place
        // of `sessions.slot0_model_id` / `slot1_model_id`.
        sqlx::query("UPDATE roles SET default_model_id = 'opus' WHERE slug = 'hands'")
            .execute(s.pool()).await.unwrap();
        sqlx::query("UPDATE roles SET default_model_id = 'sonnet' WHERE slug = 'eyes'")
            .execute(s.pool()).await.unwrap();

        assert_eq!(s.ensure_session_roster("s1", MAX_SESSION_PARTICIPANTS).await.unwrap(), 2);

        let roster = s.participants_for_session("s1").await.unwrap();
        assert_eq!(roster.len(), 2);
        // The handle is the ROLE's slug and the display name is the ROLE's, so
        // nothing in a fresh session is called Brian or Rain.
        assert_eq!(roster[0].slug, "hands");
        assert_eq!(roster[0].display_name, "HANDS");
        assert_eq!(roster[0].turn_position, 0, "HANDS acts first");
        assert_eq!(roster[0].model_id.as_deref(), Some("opus"), "the role's default model");
        assert!(roster[0].capabilities.contains("edit_files"));
        assert!(roster[0].enabled);
        assert_eq!(roster[1].slug, "eyes");
        assert_eq!(roster[1].display_name, "EYES");
        assert_eq!(roster[1].turn_position, 1);
        assert_eq!(roster[1].model_id.as_deref(), Some("sonnet"));
        assert!(!roster[1].capabilities.contains("edit_files"), "EYES stays read-only");
        // A participant without a cursor is invisibly undeliverable.
        for p in &roster {
            assert_eq!(s.cursor_for(p.id).await.unwrap(), 0);
        }

        // A role added later joins the default roster — the third participant
        // the two literal subqueries could never produce.
        s.create_session("s2", "t", None).await.unwrap();
        let auditor = s
            .create_role(&RoleDraft {
                display_name: "AUDITOR".into(),
                capabilities: "[\"read_channel\"]".into(),
                participation_mode: "active".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(s.ensure_session_roster("s2", MAX_SESSION_PARTICIPANTS).await.unwrap(), 3);
        let three = s.participants_for_session("s2").await.unwrap();
        assert_eq!(
            three.iter().map(|p| p.slug.as_str()).collect::<Vec<_>>(),
            ["hands", "eyes", "auditor"],
            "creation order, so a rename cannot reorder who acts first"
        );
        assert_eq!(three[2].role_id, Some(auditor.id));
        assert!(three.iter().all(|p| p.enabled));

        // An archived role drops out, and an on-demand one never joins (rc3 D1
        // — nothing wakes it, so it would be a seat with no process).
        s.create_session("s3", "t", None).await.unwrap();
        s.set_role_archived(auditor.id, true).await.unwrap();
        assert_eq!(s.ensure_session_roster("s3", MAX_SESSION_PARTICIPANTS).await.unwrap(), 2);
    }

    /// **The seed path obeys the cap the pick path always did** (round-2 B3).
    ///
    /// `MAX_SESSION_PARTICIPANTS` was enforced in `resolve_participant_picks`
    /// — the create DIALOG's path. The other two creation paths seed instead of
    /// picking, and reach this function: `plugin_session_create`'s `duo:true`
    /// and the external driver's `solo:false`. Both express roster size as a
    /// BOOLEAN, so neither can name a number, and neither got the cap. Seeding
    /// took *every* active non-`on_mention` role.
    ///
    /// It looked correct because the live install has three roles. The test
    /// creates more than the cap so the ceiling is the thing under test rather
    /// than an accident of how many roles happen to exist — the same reason
    /// `conventions.md` requires a fixture that can tell the candidate
    /// behaviours apart.
    #[tokio::test]
    async fn seeding_a_roster_cannot_exceed_the_participant_cap() {
        let s = Storage::memory().await.unwrap();
        // Two roles are seeded by migration; add enough to pass the cap.
        let seeded = s.list_roles().await.unwrap().len();
        for i in seeded..(MAX_SESSION_PARTICIPANTS + 3) {
            s.create_role(&RoleDraft {
                display_name: format!("ROLE{i}"),
                capabilities: "[\"read_channel\"]".into(),
                participation_mode: "active".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        }
        let active = s
            .list_roles()
            .await
            .unwrap()
            .iter()
            .filter(|r| !r.archived && r.participation_mode == MODE_ACTIVE)
            .count();
        assert!(
            active > MAX_SESSION_PARTICIPANTS,
            "fixture must exceed the cap or it proves nothing: {active} roles"
        );

        s.create_session("s-capped", "t", None).await.unwrap();
        let seeded = s.ensure_session_roster("s-capped", MAX_SESSION_PARTICIPANTS).await.unwrap();
        assert_eq!(
            seeded as usize, MAX_SESSION_PARTICIPANTS,
            "a seeded roster took {seeded} of {active} active roles — the seed \
             path is uncapped, so one boolean on the plugin wire spawns however \
             many roles happen to exist"
        );
        assert_eq!(
            s.participants_for_session("s-capped").await.unwrap().len(),
            MAX_SESSION_PARTICIPANTS
        );

        // The solo cut is unaffected — it is a different question from the
        // ceiling, and capping must not turn "one participant" into eight.
        s.create_session("s-solo", "t", None).await.unwrap();
        assert_eq!(s.ensure_session_roster("s-solo", 1).await.unwrap(), 1);
    }

    /// **The count is the roster size, not a flag for "more than one"**
    /// (round-2 B3, second half).
    ///
    /// `ensure_session_roster` took a `first_role_only: bool`, so every value
    /// above 1 collapsed to the same roster. The reviewer measured what that
    /// cost: replacing a caller's count with a hardcoded `2` left the whole
    /// suite green, because nothing anywhere distinguished 2 from 3 from 8.
    ///
    /// Asserted as a SEQUENCE of distinct sizes rather than one call, because a
    /// single-count fixture cannot tell "honours the number" from "seeds
    /// whatever exists" — the same fixture-shape rule `conventions.md` records
    /// from the three-numerators incident.
    #[tokio::test]
    async fn a_seeded_roster_is_exactly_the_size_asked_for() {
        let s = Storage::memory().await.unwrap();
        let seeded = s.list_roles().await.unwrap().len();
        for i in seeded..6 {
            s.create_role(&RoleDraft {
                display_name: format!("ROLE{i}"),
                capabilities: "[\"read_channel\"]".into(),
                participation_mode: "active".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        }

        for wanted in [1usize, 2, 3, 5] {
            let sid = format!("s-{wanted}");
            s.create_session(&sid, "t", None).await.unwrap();
            assert_eq!(
                s.ensure_session_roster(&sid, wanted).await.unwrap() as usize,
                wanted,
                "asked for {wanted} participants"
            );
            assert_eq!(
                s.participants_for_session(&sid).await.unwrap().len(),
                wanted,
                "roster rows for {wanted}"
            );
        }

        // More than the install has yields what it has — this is the seed path,
        // whose callers ask for a default rather than name a roster, so there is
        // nothing useful to fail for.
        s.create_session("s-greedy", "t", None).await.unwrap();
        let active = s
            .list_roles()
            .await
            .unwrap()
            .iter()
            .filter(|r| !r.archived && r.participation_mode == MODE_ACTIVE)
            .count();
        assert_eq!(
            s.ensure_session_roster("s-greedy", MAX_SESSION_PARTICIPANTS)
                .await
                .unwrap() as usize,
            active
        );
    }

    /// Two participants of ONE role are both addressable — the collision rule
    /// [`participant_slug`] documents, exercised through the real seed.
    #[tokio::test]
    async fn two_participants_of_one_role_get_distinct_handles() {
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        s.seed_session_roster(
            "s1",
            &[
                ParticipantDraft { role_id: hands.id, ..Default::default() },
                ParticipantDraft { role_id: hands.id, ..Default::default() },
                ParticipantDraft { role_id: hands.id, ..Default::default() },
            ],
        )
        .await
        .unwrap();
        let roster = s.participants_for_session("s1").await.unwrap();
        assert_eq!(
            roster.iter().map(|p| p.slug.as_str()).collect::<Vec<_>>(),
            ["hands", "hands-2", "hands-3"],
            "the second and third participant of a role must still be addressable"
        );
        // Every one of them resolves back through the handle a mention parses.
        for p in &roster {
            assert_eq!(
                s.participant_by_slug("s1", &p.slug).await.unwrap().map(|q| q.id),
                Some(p.id)
            );
        }
        // Their display names are identical BY DESIGN — the display rule is
        // role + model, and these share both. The slug is the tiebreaker.
        assert!(roster.iter().all(|p| p.display_name == "HANDS"));
    }

    #[tokio::test]
    async fn seeding_a_roster_twice_is_a_no_op() {
        // It runs pre-spawn on EVERY respawn, so non-idempotence would mean a
        // duplicate roster per restart.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        assert_eq!(s.ensure_session_roster("s1", MAX_SESSION_PARTICIPANTS).await.unwrap(), 2);
        assert_eq!(s.ensure_session_roster("s1", MAX_SESSION_PARTICIPANTS).await.unwrap(), 0, "second call inserts nothing");
        assert_eq!(s.participants_for_session("s1").await.unwrap().len(), 2);
    }

    /// **The product default for every create path with no dialog (rc3 D13).**
    ///
    /// One participant — the FIRST active role by `roles.id` — not N rows with
    /// the extras disabled, which is what this did while the roster was a fixed
    /// pair. There is no setting behind it: `rain_disabled_default` is deleted,
    /// so this assertion IS the default, and the external driver and the plugin
    /// create arm both land on it.
    #[tokio::test]
    async fn the_dialogless_default_seeds_exactly_one_participant() {
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        assert_eq!(s.ensure_session_roster("s1", 1).await.unwrap(), 1);
        let roster = s.participants_for_session("s1").await.unwrap();
        assert_eq!(roster.len(), 1, "no row for a role nobody invited");
        assert!(roster[0].enabled, "the one participant runs");
        assert_eq!(
            roster[0].slug, "hands",
            "the FIRST active role by roles.id — creation order, not alphabetical"
        );
        // And it can take a turn, which is what makes it a session rather than
        // a roster of one that nothing wakes.
        assert!(s.next_active_participant("s1", None).await.unwrap().is_some());
    }

    // ---- rc3: N participants chosen from roles ---------------------------

    /// Every column of a session's roster, rendered as text, in ring order.
    ///
    /// `quote()` renders NULL as `NULL` and every other value as a SQL literal,
    /// so the comparison is type-independent and total — which is the point:
    /// this is the oracle the parity claim rests on, and a comparison that
    /// silently skipped a column would let the two paths diverge in exactly the
    /// place nobody looked. `id` and `session_id` are excluded because they
    /// necessarily differ between two sessions; everything else must match.
    ///
    /// `joined_at` is rendered as a RELATION rather than a timestamp — both
    /// paths write the session's own `created_at`, and two sessions are created
    /// milliseconds apart, so comparing the literal would compare the clock.
    /// A row that joined at some OTHER time still renders its literal and still
    /// fails the comparison.
    async fn roster_verbatim(s: &Storage, session_id: &str) -> Vec<Vec<String>> {
        use sqlx::Row as _;
        let (created_at,): (String,) = sqlx::query_as("SELECT created_at FROM sessions WHERE id = ?")
            .bind(session_id)
            .fetch_one(&s.pool)
            .await
            .unwrap();
        let joined_at_is_created_at = format!("joined_at='{created_at}'");
        let columns: Vec<String> = sqlx::query_as("SELECT name FROM pragma_table_info(?)")
            .bind("session_participants")
            .fetch_all(&s.pool)
            .await
            .unwrap()
            .into_iter()
            .map(|(name,): (String,)| name)
            .collect();
        // A column added later joins the comparison automatically — but only if
        // this list is what the table actually holds, so pin it. If this fails,
        // the fix is to decide whether the new column is part of roster parity
        // (it almost certainly is) and update the list, not to loosen it.
        assert_eq!(
            columns,
            [
                "id", "session_id", "slug", "display_name", "role_id", "model_id", "runtime",
                "capabilities", "participation_mode", "prompt", "turn_position", "done_vote",
                "effort", "ultracode", "claude_session_id", "enabled", "joined_at", "left_at",
                // rc3 D20 (migration 0052). IS part of roster parity: both paths
                // must write NULL when nobody picked a colour, and a default
                // roster that quietly differed here would give the dialog's
                // sessions a different rotation from the driver's.
                "color",
                // rc3 D20's other half (migration 0053), and part of parity for
                // exactly the same reason: a default roster that wrote anything
                // but NULL here would name participants the user never named.
                "label",
            ],
            "session_participants grew a column; roster parity has to cover it"
        );
        let compared: Vec<&String> = columns
            .iter()
            .filter(|c| c.as_str() != "id" && c.as_str() != "session_id")
            .collect();
        let selected = compared
            .iter()
            .map(|c| format!("quote({c})"))
            .collect::<Vec<_>>()
            .join(", ");
        let rows = sqlx::query(&format!(
            "SELECT {selected} FROM session_participants \
             WHERE session_id = ? ORDER BY turn_position, id"
        ))
        .bind(session_id)
        .fetch_all(&s.pool)
        .await
        .unwrap();
        rows.iter()
            .map(|row| {
                compared
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let rendered = format!("{c}={}", row.get::<String, _>(i));
                        if rendered == joined_at_is_created_at {
                            "joined_at=<the session's created_at>".to_string()
                        } else {
                            rendered
                        }
                    })
                    .collect()
            })
            .collect()
    }

    /// The two drafts that reproduce today's roster, given the seeded roles.
    async fn hands_and_eyes_drafts(s: &Storage) -> Vec<ParticipantDraft> {
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        let eyes = s.role_by_slug("eyes").await.unwrap().unwrap();
        vec![
            ParticipantDraft {
                role_id: hands.id,
                model_id: Some("opus".into()),
                effort: Some("max".into()),
                ultracode: Some(true),
                color: None,
                label: None,
            },
            ParticipantDraft {
                role_id: eyes.id,
                model_id: Some("sonnet".into()),
                effort: Some("low".into()),
                ultracode: Some(false),
                color: None,
                label: None,
            },
        ]
    }

    #[tokio::test]
    async fn n_of_two_is_byte_identical_to_the_default_roster() {
        // THE reframe test. rc3 moves where a roster comes from — two literal
        // `WHERE slug = 'hands' / 'eyes'` subqueries become the user's picks —
        // and moving a source is only a reframe if the result is the same. So
        // this builds one session each way, from the same inputs, and compares
        // EVERY column of both rosters.
        let s = storage_with_0044().await;

        // The default way: no dialog, so the roster comes from the roles and the
        // caller's per-slot picks are laid onto the rows it produced — exactly
        // what `core::session::seed_default_roster` does for every path that has
        // no participant list.
        s.create_session("s-old", "t", None).await.unwrap();
        assert_eq!(s.ensure_session_roster("s-old", MAX_SESSION_PARTICIPANTS).await.unwrap(), 2);
        let seeded = s.participants_for_session("s-old").await.unwrap();
        for (p, (model, effort, ultracode)) in seeded.iter().zip([
            ("opus", "max", true),
            ("sonnet", "low", false),
        ]) {
            s.set_participant_model(p.id, Some(model)).await.unwrap();
            s.set_participant_spawn_knobs(p.id, Some(effort), Some(ultracode))
                .await
                .unwrap();
        }

        // The new way: the same two roles, chosen.
        s.create_session("s-new", "t", None).await.unwrap();
        let drafts = hands_and_eyes_drafts(&s).await;
        let ids = s.seed_session_roster("s-new", &drafts).await.unwrap();
        assert_eq!(ids.len(), 2);

        let old = roster_verbatim(&s, "s-old").await;
        let new = roster_verbatim(&s, "s-new").await;
        assert_eq!(old.len(), 2, "precondition: the default roster is two rows");
        assert_eq!(
            new, old,
            "a session created from the HANDS and EYES roles must be the session bot-hq \
             has always created"
        );
        // And the invariant the columns cannot show: a participant with no
        // cursor is invisibly undeliverable.
        for id in ids {
            assert_eq!(s.cursor_for(id).await.unwrap(), 0);
        }
    }

    #[tokio::test]
    async fn one_participant_is_a_roster_of_one_and_the_ring_runs() {
        // Design §1's default. The ring has to hand this participant the turn,
        // hand it the turn again after its own, and halt only on its vote —
        // otherwise "one agent" is a session that looks staffed and never acts.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        let ids = s
            .seed_session_roster(
                "s1",
                &[ParticipantDraft {
                    role_id: hands.id,
                    ..Default::default()
                }],
            )
            .await
            .unwrap();
        assert_eq!(ids.len(), 1);

        let roster = s.participants_for_session("s1").await.unwrap();
        assert_eq!(roster.len(), 1, "one pick is one row — no disabled placeholder");
        assert_eq!(roster[0].turn_position, 0);
        assert!(roster[0].enabled);
        assert!(roster[0].capabilities.contains("edit_files"), "HANDS still executes");

        let first = s.next_active_participant("s1", None).await.unwrap().unwrap();
        assert_eq!(first.id, ids[0], "a user message opens on the only participant");
        let next = s.next_active_participant("s1", Some(&first)).await.unwrap();
        assert_eq!(next.map(|p| p.id), Some(ids[0]), "the ring wraps onto itself");
        assert!(!s.all_active_voted_done("s1").await.unwrap(), "nobody has voted yet");
        s.set_done_vote(ids[0], true).await.unwrap();
        assert!(s.all_active_voted_done("s1").await.unwrap(), "its vote alone ends the cycle");
    }

    #[tokio::test]
    async fn turn_slots_are_the_order_given_and_are_unique() {
        // The list IS the running order: the ring steps by place in the
        // rotation, so seeding EYES first would make the reviewer speak before
        // there is anything to review. Reversed here so the assertion cannot
        // pass off the roles' own ids or their alphabetical order.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let mut drafts = hands_and_eyes_drafts(&s).await;
        drafts.reverse();
        s.seed_session_roster("s1", &drafts).await.unwrap();

        let roster = s.participants_for_session("s1").await.unwrap();
        let slots: Vec<(i64, &str)> = roster
            .iter()
            .map(|p| (p.turn_position, p.slug.as_str()))
            .collect();
        assert_eq!(slots, [(0, "eyes"), (1, "hands")], "slot 0 is whoever was listed first");
        // 0045's partial unique index is what makes a shared slot
        // unrepresentable; this proves the seeding path does not need it.
        let mut seen: Vec<i64> = roster.iter().map(|p| p.turn_position).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), roster.len(), "no two participants share a slot");
    }

    #[tokio::test]
    async fn a_participant_model_overrides_the_roles_default() {
        // rc3 D8: the Roles tab names a default model, the New Session dialog
        // overrides it per participant. Both halves, on one roster — the second
        // participant takes what the role names because it asked for nothing.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        s.update_role(
            hands.id,
            &RoleDraft {
                display_name: hands.display_name.clone(),
                slug: None,
                description_prompt: hands.description_prompt.clone(),
                capabilities: hands.capabilities.clone(),
                participation_mode: hands.participation_mode.clone(),
                default_model_id: Some("role-default".into()),
            },
        )
        .await
        .unwrap();

        s.seed_session_roster(
            "s1",
            &[
                ParticipantDraft {
                    role_id: hands.id,
                    model_id: Some("chosen-at-invite".into()),
                    ..Default::default()
                },
                ParticipantDraft {
                    role_id: hands.id,
                    model_id: None,
                    ..Default::default()
                },
            ],
        )
        .await
        .unwrap();

        let roster = s.participants_for_session("s1").await.unwrap();
        assert_eq!(
            roster[0].model_id.as_deref(),
            Some("chosen-at-invite"),
            "the participant's own pick wins"
        );
        assert_eq!(
            roster[1].model_id.as_deref(),
            Some("role-default"),
            "no pick falls back to the role's default"
        );
    }

    #[tokio::test]
    async fn a_chosen_roster_survives_the_next_spawn() {
        // `ensure_session_roster` runs pre-spawn on EVERY session, and its
        // `OR IGNORE` idempotence keys on the slug. A one-participant roster
        // collides on `brian` and would have sailed straight through the second
        // insert — handing the user a Rain they did not invite.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let hands = s.role_by_slug("hands").await.unwrap().unwrap();
        s.seed_session_roster(
            "s1",
            &[ParticipantDraft {
                role_id: hands.id,
                ..Default::default()
            }],
        )
        .await
        .unwrap();
        let before = roster_verbatim(&s, "s1").await;

        assert_eq!(s.ensure_session_roster("s1", MAX_SESSION_PARTICIPANTS).await.unwrap(), 0, "nothing to seed");
        assert_eq!(s.ensure_session_roster("s1", MAX_SESSION_PARTICIPANTS).await.unwrap(), 0, "still nothing on respawn");

        assert_eq!(roster_verbatim(&s, "s1").await, before, "the chosen roster is untouched");
    }

    #[tokio::test]
    async fn a_session_created_before_rc3_still_opens_with_the_same_roster() {
        // The other half of parity: every session in the live database was
        // created the old way, and opening one calls `ensure_session_roster`.
        // A 0044-backfilled roster must come back byte for byte, and a session
        // from the rosterless window must still be healed into the SAME shape a
        // pre-rc3 open would have given it.
        let s = storage_with_0044().await;

        // A session as 0044 left it: both rows, seeded before this change.
        s.create_session("s-backfilled", "t", None).await.unwrap();
        s.ensure_session_roster("s-backfilled", MAX_SESSION_PARTICIPANTS).await.unwrap();
        let backfilled = roster_verbatim(&s, "s-backfilled").await;
        assert_eq!(s.ensure_session_roster("s-backfilled", MAX_SESSION_PARTICIPANTS).await.unwrap(), 0);
        assert_eq!(
            roster_verbatim(&s, "s-backfilled").await,
            backfilled,
            "re-opening an existing session must not reshape its roster"
        );

        // A session from the window where nothing seeded a roster at all.
        s.create_session("s-rosterless", "t", None).await.unwrap();
        s.post_to_channel("s-rosterless", "participant", Some("hands"), "text", "work", None)
            .await
            .unwrap();
        assert_eq!(
            s.ensure_session_roster("s-rosterless", MAX_SESSION_PARTICIPANTS).await.unwrap(),
            2,
            "the heal path still seeds both"
        );
        let healed = s.participants_for_session("s-rosterless").await.unwrap();
        assert_eq!(healed.len(), 2);
        assert_eq!(healed[0].slug, "hands");
        assert_eq!(healed[1].slug, "eyes");
        let rows = all_rows(&s, "s-rosterless").await;
        assert_eq!(
            rows[0].participant_id,
            Some(healed[0].id),
            "and still repairs what that window wrote"
        );
    }

    #[tokio::test]
    async fn a_roster_is_seeded_once_and_refuses_what_it_cannot_write() {
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let drafts = hands_and_eyes_drafts(&s).await;

        assert!(
            s.seed_session_roster("s1", &[]).await.is_err(),
            "an empty roster is a session nobody can act in"
        );
        // Two participants of the SAME role no longer clash — rc3 D10 derives
        // the second one a `<role>-2` handle. Proved in
        // `two_participants_of_one_role_get_distinct_handles`.
        let mut unknown = drafts.clone();
        unknown[0].role_id = 9999;
        assert!(
            s.seed_session_roster("s1", &unknown).await.is_err(),
            "a role that does not exist is not a participant"
        );
        assert!(
            s.seed_session_roster("s-missing", &drafts).await.is_err(),
            "a roster needs a session to belong to"
        );
        // Every refusal above is decided before the first INSERT, so a refused
        // create leaves a session with no roster rather than a partial one.
        assert!(
            s.participants_for_session("s1").await.unwrap().is_empty(),
            "a refused seed writes no rows"
        );

        s.seed_session_roster("s1", &drafts).await.unwrap();
        // The second seed the SCHEMA cannot stop, which is what makes the guard
        // load-bearing rather than decorative: a fresh slug clears
        // `UNIQUE (session_id, slug)`, and an `on_mention` role is outside
        // 0045's turn-slot index (`WHERE enabled <> 0 AND participation_mode =
        // 'active'`), so both constraints let this through and the roster
        // quietly grows a third member.
        let summonable = s
            .create_role(&RoleDraft {
                display_name: "Watcher".into(),
                capabilities: "[]".into(),
                participation_mode: "on_mention".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let third = ParticipantDraft {
            role_id: summonable.id,
            ..Default::default()
        };
        assert!(
            s.seed_session_roster("s1", &[third]).await.is_err(),
            "seeding twice would grow the roster, not merge into it"
        );
        assert_eq!(s.participants_for_session("s1").await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn seeding_repairs_messages_written_before_the_roster() {
        // The live defect: a post-0044 session logged 60 messages with
        // participant_id NULL before anything created its roster. Seeding must
        // map them, or that history is permanently unattributed in the channel.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.post_to_channel("s1", "participant", Some("hands"), "text", "work", None)
            .await
            .unwrap();
        s.post_to_channel("s1", "participant", Some("eyes"), "text", "review", None)
            .await
            .unwrap();
        s.insert_message("s1", MessageKind::Text, "reply").await.unwrap();
        let before = all_rows(&s, "s1").await;
        assert!(before.iter().all(|m| m.participant_id.is_none()), "precondition: unmapped");

        s.ensure_session_roster("s1", MAX_SESSION_PARTICIPANTS).await.unwrap();

        let roster = s.participants_for_session("s1").await.unwrap();
        let after = all_rows(&s, "s1").await;
        assert_eq!(after[0].participant_id, Some(roster[0].id), "HANDS' row mapped");
        assert_eq!(after[1].participant_id, Some(roster[1].id), "EYES' row mapped");
        assert_eq!(after[2].participant_id, None, "a user row has no participant");
        assert_eq!(after[2].origin, "user");
    }

    #[tokio::test]
    async fn insert_message_resolves_the_participant_once_the_roster_exists() {
        // Closes the loop with B4a's dual-write: seeded roster → the inline
        // subquery resolves, so nothing new accumulates unmapped.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        s.ensure_session_roster("s1", MAX_SESSION_PARTICIPANTS).await.unwrap();
        s.post_to_channel("s1", "participant", Some("hands"), "text", "work", None)
            .await
            .unwrap();
        let roster = s.participants_for_session("s1").await.unwrap();
        let rows = all_rows(&s, "s1").await;
        assert_eq!(rows[0].participant_id, Some(roster[0].id));
        assert_eq!(rows[0].origin, "participant");

        // rc3 D10, stated rather than assumed: a LEGACY row authored `brian`
        // does not resolve against a role-derived roster, and is not meant to.
        // It keeps its author string and renders as history.
        //
        // **The retired slug is deliberate here and must not be swept.** This
        // fixture's entire subject is a row written under a name no role
        // answers to; replacing it with `hands` makes the row resolve and the
        // test asserts nothing. Same category as the dated incident labels in
        // the CL — a retired name used to NAME the retired thing.
        s.post_to_channel("s1", "participant", Some("brian"), MessageKind::Text.as_str(), "legacy", None)
            .await
            .unwrap();
        let rows = all_rows(&s, "s1").await;
        assert_eq!(rows[1].participant_id, None, "legacy history is not backfilled");
        assert_eq!(
            s.messages_for_session("s1", None).await.unwrap()[1].author,
            "brian",
            "and it still renders under the name it was written with"
        );
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
        s.ensure_session_roster("s1", MAX_SESSION_PARTICIPANTS).await.unwrap();
        let pm = s
            .post_to_channel("s1", "participant", Some("hands"), "text", "work",
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
            format!(
                "[{}] {}",
                speaker_of(
                    &rows[0].origin,
                    rows[0].author.as_deref(),
                    rows[0].speaker_label.as_deref(),
                ),
                render_wire(rows[0].envelope.as_ref(), &rows[0].content)
            )
        );
        // Spelled out, because "who wrote it" is the half a participant cannot
        // work out for itself (rc3 D23).
        assert_eq!(pm.speaker(), "hands");
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
            "[system] [PHASE: Verify]\n⚠ 3 unresolved EYES blocking finding(s) — run \
             check_open_findings and disposition each (fix/rebut) before you \
             commit.\ndeclare state"
        );
        // The scope survives the round trip, or `send_to_all`'s check would wave
        // every replayed row through.
        assert_eq!(replayed.session_id(), "s1");
    }

    /// **s-f6a441ff: no single row may be able to blow a context window.** A
    /// 2,977,078-byte user paste rode one batch into both participants; every
    /// prompt after it exceeded the model window and the session died volleying
    /// "Prompt is too long". The wire — the ONE place rows become stdin bytes —
    /// clamps any oversized body and tells the reading agent what was cut.
    #[tokio::test]
    async fn an_oversized_body_is_clamped_on_the_wire_but_whole_on_the_record() {
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        let giant = "x".repeat(WIRE_BODY_CLAMP_BYTES * 3);
        let pm = s
            .post_to_channel("s1", "user", None, MessageKind::Text.as_str(), &giant, None)
            .await
            .unwrap();

        // The record keeps every byte — the clamp is a delivery decision.
        assert_eq!(all_rows(&s, "s1").await[0].content.len(), giant.len());

        let wire = pm.wire();
        assert!(
            wire.len() < WIRE_BODY_CLAMP_BYTES + 600,
            "the wire is bounded near the cap, got {} bytes",
            wire.len()
        );
        assert!(
            wire.contains("truncated on delivery"),
            "the reading agent is told the body was cut"
        );
        assert!(
            wire.contains(&giant.len().to_string()),
            "…and how large the original is"
        );
        // The batch form inherits the clamp — it is the same wire() per row.
        assert!(
            PersistedMessage::wire_batch(std::slice::from_ref(&pm)).len()
                < WIRE_BODY_CLAMP_BYTES + 600
        );
    }

    #[test]
    fn the_wire_clamp_cuts_at_a_char_boundary() {
        // A cap landing mid-codepoint must step back, not panic: after the
        // single-byte prefix every 'é' straddles an even offset, so the even
        // cap is guaranteed to land inside one.
        let body = format!("x{}", "é".repeat(WIRE_BODY_CLAMP_BYTES)); // ≈2× the cap
        let clamped = clamped_body(&body);
        assert!(clamped.len() < body.len());
        assert!(clamped.contains("truncated on delivery"));
    }

    #[test]
    fn a_body_at_the_cap_is_untouched() {
        // The user-message gate admits up to the SAME constant, so an accepted
        // message must never arrive truncated — the boundary case is exact.
        let body = "x".repeat(WIRE_BODY_CLAMP_BYTES);
        assert!(matches!(clamped_body(&body), std::borrow::Cow::Borrowed(_)));
    }

    #[tokio::test]
    async fn a_session_with_no_active_participants_is_already_done() {
        // A session of nothing but summonable participants must not wedge the
        // sequencer on an unwrap —
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
        // cycling. Reached by an all-`on_mention` roster, by every active
        // participant being disabled, and by a session with no roster yet — all
        // three covered below or by the roster tests.
        let s = storage_with_0044().await;
        s.create_session("s1", "t", None).await.unwrap();
        // `on_mention` is skipped in the rotation and woken only when the user
        // summons it, so a roster of nothing but these is an empty rotation.
        s.insert_participant("s1", "o", "O", None, None, "[]", "on_mention", 0).await.unwrap();
        s.insert_participant("s1", "d", "D", None, None, "[]", "on_mention", 1).await.unwrap();
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
        // one a user can reach from the UI rather than by building a roster of
        // nothing but summonable participants.
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
