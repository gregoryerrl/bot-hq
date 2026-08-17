import { useMemo } from "react";
import { useTauriQuery } from "../hooks/useInvoke";
import { colorByName, participantHue } from "../components/authorColor";

/**
 * How a participant is NAMED in the UI, and nothing else.
 *
 * rc3 D10: a participant is displayed as its ROLE and its MODEL — never as an
 * agent's person-name. The user's words: *"I already said multiple times that
 * I'm dropping the names, only the Role + Model Name."* The names that used to
 * appear here survive only as internal keys (message authors, store maps,
 * `claude-overrides.json` scopes); this module is the boundary they stop at.
 *
 * ---
 * ## The hand-written mirror of `ParticipantView`
 *
 * Must stay in step with `pub struct ParticipantView` in
 * `src/tauri_cmd/sessions.rs`, which `list_session_participants` returns.
 *
 * Hand-written on purpose, and it stays that way: `frontend/src/lib/bindings.ts`
 * is `@ts-nocheck` and regenerates only at app launch (or via `cargo run --
 * export-bindings`), so importing the contract from there would put the
 * frontend's only *checked* declaration of it behind a no-check barrier and
 * leave a fresh clone type-checking against whatever was last committed. This is
 * the house pattern for every Rust view type the frontend reads.
 *
 * This block used to say the opposite — "when the two units merge, delete
 * `ParticipantView` from this file and import it from `../lib/bindings`". The
 * merge landed and that step was never run, which is how the mirror sat three
 * fields short of the contract for long enough for `label` to go unrendered
 * (round-4 F1). The instruction was the stale artifact, not the mirror.
 */
export type ParticipantView = {
  id: number;
  /** Internal key (message author, store map key). **NEVER displayed.** */
  slug: string;
  /** e.g. `"HANDS"`. Null when the role row is gone. */
  role_display_name: string | null;
  /** e.g. `"Claude Opus 5"`. Null when no model is set. */
  model_display_name: string | null;
  turn_position: number;
  /** `"active"` | `"on_mention"` (rc3 D17/D18). */
  participation_mode: string;
  /** The user's colour pick by palette NAME, or null to take the rotation
   *  (rc3 D20). */
  color: string | null;
  /** The user's own name for this participant, or null to take the role and
   *  ordinal (rc3 **D20**, migration 0053). See {@link participantLabel}. */
  label: string | null;
  /** This participant's own effort / ultracode pick (rc3 **D12**), or null to
   *  inherit. This is the CHOICE, not the effective value — see the pair below,
   *  and {@link SpawnBadge} for which one is rendered and which one is styled. */
  effort: string | null;
  ultracode: boolean | null;
  /** What this participant was ACTUALLY spawned with (migration 0061): the pair
   *  left standing after the precedence chain and its exclusion rule, recorded
   *  at spawn because it cannot be recomputed here — `claude-overrides.json`
   *  keys by ROLE SLUG, which this view does not carry — and re-resolving it
   *  would answer "what it would be spawned with NOW". */
  effort_at_spawn: string | null;
  ultracode_at_spawn: boolean | null;
  /** Whether the pair above describes a real spawn. The common path reconciles
   *  to null, so without this "spawned with no override in force" and "this row
   *  predates 0061" are the same two nulls. False means say nothing. */
  spawn_knobs_recorded: boolean;
  enabled: boolean;
};

/** The roster read (`src/tauri_cmd/sessions.rs`). Named once, here. */
const LIST_PARTICIPANTS_CMD = "list_session_participants";

/**
 * The contract's display rule, in one function:
 *
 * > The user's `label`, else `role_display_name` with its ordinal. Then
 * > ` · model_display_name` when there is a model. When the first half is
 * > absent fall back to the model alone; when both are absent, the slug.
 *
 * **The label replaces the role-and-ordinal half, and only that half** (rc3
 * **D20**, migration 0053). The model suffix survives it, because what a
 * participant RUNS is a different fact from what the user named it.
 *
 * Blank is not a name: an empty or whitespace label falls back to the ordinal
 * rather than rendering an empty byline.
 *
 * The slug fallback is the ONLY path that can put an internal key on screen,
 * and it only fires when there is nothing else to say.
 *
 * **Must stay in step with `participant_display_name` in
 * `src/storage/participants.rs`** — same rule, same case table, two surfaces.
 * The label branch was missing here until round-4 F1, so a user who named a
 * participant saw that name in the agent's own prompt roster
 * (`core/session.rs::resolve_roster_facts`) and in session-doc attribution,
 * and never anywhere in the UI: two surfaces asserting different identities
 * for one row.
 */
export function participantLabel(
  p: Pick<
    ParticipantView,
    "slug" | "role_display_name" | "model_display_name" | "label"
  >,
): string {
  const ordinal = slugOrdinal(p.slug);
  const named = p.label?.trim() || null;
  const bare = p.role_display_name?.trim() || null;
  const role = named ?? (bare && ordinal ? `${bare}-${ordinal}` : bare);
  const model = p.model_display_name?.trim() || null;
  if (role && model) return `${role} · ${model}`;
  if (role) return role;
  if (model) return model;
  return p.slug;
}

/**
 * The `-N` a duplicate slug carries: `eyes-2` → `2`, `eyes` → `null`
 * (rc3 **D20**).
 *
 * **Two participants of one role rendered identically**, character for
 * character — `EYES · DeepSeek V4 Pro` twice, in the same colour, because the
 * label had no ordinal and `authorColor` hashes the label. Reported from a live
 * N=3 run: *"for the 2 reviewers, i don't know which is which."*
 *
 * Taken from the SLUG rather than counted over the roster so the visible name
 * and the internal key agree by construction — the backend assigns `eyes`,
 * `eyes-2`, `eyes-3` at invite time. A second numbering would disagree with the
 * first the moment a participant is disabled.
 *
 * Must stay in step with `participant_display_name` in
 * `src/storage/participants.rs`; the two render the same participant on
 * different surfaces.
 */
export function slugOrdinal(slug: string): number | null {
  const at = slug.lastIndexOf("-");
  if (at <= 0) return null;
  const tail = slug.slice(at + 1);
  if (!/^\d+$/.test(tail)) return null;
  const n = Number(tail);
  return n >= 2 ? n : null;
}

/**
 * Authors that are not participants. They pass through untouched — a chat line
 * from the user is not a role, and inventing one for it would be its own lie.
 */
const NON_PARTICIPANT_AUTHORS: Record<string, string> = {
  user: "You",
  system: "System",
};

/**
 * What an author with no roster row reads as.
 *
 * It used to be the author slug itself, on the reasoning that "the line still
 * has to be attributable". That reasoning survives; the slug does not. rc3 D10
 * kept legacy rows renderable on purpose — *"brian and rain's history can be
 * legacy data"* — and every one of those rows carries `author = 'brian'` or
 * `'rain'`, so falling back to the slug puts exactly the two names the same
 * decision removed back on screen: *"I don't want to see brian and rain anymore
 * moving forward."*
 *
 * So an unresolvable author is named by what is actually known about it, which
 * is nothing. It stays visibly attributed — the byline, the tag and the status
 * line all still render — without asserting an identity the roster cannot back.
 */
// Re-exported from a leaf module so `components/authorColor` can have it
// without importing THIS file, which would close an import cycle — see
// `participantNames.ts` for the blank window that produced.
export { UNKNOWN_PARTICIPANT } from "./participantNames";
import { UNKNOWN_PARTICIPANT } from "./participantNames";

/**
 * The key a SLOT-SHAPED runtime field lands under.
 *
 * Two of the backend's runtime payloads are still shaped as a fixed pair —
 * `SessionActivityEvent { slot0_busy, slot1_busy }` and
 * `SessionRuntime { slot0_health, slot1_health }`. Those field names are frozen
 * wire that names **turn slots, not agents**: `src/core/activity.rs` fills them
 * from `slugs.get(0)` / `slugs.get(1)`, and `src/tauri_cmd/sessions.rs` from
 * `handle.participants.get(0)` / `.get(1)`.
 *
 * The frontend used to unpack them under the literal keys `"brian"` / `"rain"`,
 * which no rc3 roster has — so every lookup keyed by a roster slug missed, the
 * mount backfill left every health dot blank, and the turn-status line printed
 * the raw key.
 *
 * The `#` prefix is what keeps the two spaces from overwriting each other in
 * one map: `slugify` (`src/storage/participants.rs`) emits `[a-z0-9-]` only,
 * trimmed of leading dashes and never empty, so no slug can start with `#`.
 *
 * The argument is a SPAWN SLOT, not a `turn_position` — see
 * {@link spawnSlotOf} for why those are not the same number.
 */
export function slotKey(spawnSlot: number): string {
  return `#slot${spawnSlot}`;
}

/**
 * Does this participant get a claude-code subprocess?
 *
 * Mirrors `spawnable` in `src/core/session.rs` exactly — `enabled` and not
 * `on_demand` — because that filter is what decides which participants the
 * slot-shaped payloads describe. Observers ARE spawned; they read the channel
 * and may post, they simply never take a scheduled turn.
 */
function isSpawnable(
  p: Pick<ParticipantView, "enabled" | "participation_mode">,
): boolean {
  return p.enabled && p.participation_mode !== "on_demand";
}

/**
 * The slot index the backend's fixed-pair payloads report this participant
 * under, or `null` when it occupies no slot.
 *
 * **This is an index into the SPAWNABLE roster, not `turn_position`.** Both
 * producers build their pair off `spawnable(roster)`:
 * `spawn_session_handle` (`src/core/session.rs`) stores that filtered vec as
 * `SessionHandle.participants`, which `get_session_runtime` indexes with
 * `.get(0)` / `.get(1)`, and hands the same slugs to `ActivityTracker::new`,
 * which indexes them with `slugs.get(0)` / `.get(1)`.
 *
 * `turn_position` is the roster row's own column and counts every row —
 * `insert_roster` writes it as the un-filtered enumerate index. The two numbers
 * agree only while every row is spawnable. They part the moment one is not, and
 * then reading `slotKey(turn_position)` hands one participant's health dot,
 * context meter and busy label to a different participant: with a disabled row
 * at position 0, the backend's slot 0 IS the row at position 1, and position 1's
 * own key matches nothing.
 *
 * Derived from the roster rather than corrected by an offset so there is one
 * rule, written the same way on both sides.
 *
 * Two limits, both inherent rather than approximations:
 *   * the roster is read live while `SessionHandle.participants` is frozen at
 *     spawn, so a roster edited mid-session is only right again after a
 *     respawn — which is when the backend's own slots move too;
 *   * `participants` must arrive in `(turn_position, id)` order, which is what
 *     `participants_for_session` orders by and `participant_views` preserves.
 *     Re-sorting here would be a second source of truth, not a safeguard.
 */
export function spawnSlotOf(
  roster: readonly ParticipantView[],
  p: Pick<ParticipantView, "id">,
): number | null {
  const slot = roster.filter(isSpawnable).findIndex((q) => q.id === p.id);
  return slot < 0 ? null : slot;
}

/**
 * Every key one participant's runtime state can arrive under, most specific
 * first.
 *
 * **This is the only declaration of the two key spaces**, and both directions
 * route through it: {@link participantRuntime} reads a runtime map with it and
 * {@link participantLabelIndex} writes the label index with it. Producers that
 * key by the live slug (`session:agent_health`, `session:agent_context` — both
 * emitted with `cfg.slug`) and producers that key by turn slot (the
 * `session:activity` busy flags, the `get_session_runtime` backfill) therefore
 * resolve to the same participant without either consumer knowing which one it
 * got.
 *
 * The slug wins: it comes from a live event, the slot key from a snapshot.
 *
 * A participant with no slot gets its slug alone. That covers both a
 * non-spawnable row — nothing runs for it, so no producer can report it, and
 * claiming a slot would be claiming another participant's state — and a third
 * spawnable one, since the fixed pair reports slots 0 and 1 only. Either way its
 * slug still resolves whatever the live events supply.
 */
function participantRuntimeKeys(
  roster: readonly ParticipantView[],
  p: ParticipantView,
): string[] {
  const slot = spawnSlotOf(roster, p);
  return slot === null ? [p.slug] : [p.slug, slotKey(slot)];
}

/**
 * One participant's entry in a per-participant runtime map (health, context
 * occupancy, busy flags), looked up across both key spaces.
 *
 * Takes the whole roster because the slot key is a property of the participant's
 * PLACE in it, not of the row alone — see {@link spawnSlotOf}.
 *
 * `undefined` means "nothing reported for this participant", which every caller
 * already treats as unknown rather than empty.
 */
export function participantRuntime<T>(
  map: Record<string, T | undefined> | undefined,
  roster: readonly ParticipantView[],
  p: ParticipantView,
): T | undefined {
  if (!map) return undefined;
  for (const key of participantRuntimeKeys(roster, p)) {
    const value = map[key];
    if (value !== undefined) return value;
  }
  return undefined;
}

/**
 * Live roster for a session, in turn order.
 *
 * `enabled` is left to the caller: the session header wants to show a disabled
 * participant (greyed), while the author lookup wants every row it can get.
 */
export function useSessionParticipants(sessionId: string) {
  const { data = [], ...rest } = useTauriQuery<ParticipantView[]>(
    LIST_PARTICIPANTS_CMD,
    { sessionId },
    { enabled: !!sessionId },
  );
  return { participants: data, ...rest };
}

/**
 * Roster as a key → display-label map, for surfaces that hold a key rather than
 * a participant row: an author slug (chat messages, the Quickview, tray cards,
 * the enforcement log) or a slot key (the turn-status line's busy flags).
 *
 * Both spaces land in one map via {@link participantRuntimeKeys}, so
 * {@link authorLabel} is the single lookup every one of those surfaces uses and
 * they cannot drift apart.
 */
export function participantLabelIndex(
  participants: readonly ParticipantView[],
): Record<string, string> {
  const out: Record<string, string> = {};
  for (const p of participants) {
    const label = participantLabel(p);
    for (const key of participantRuntimeKeys(participants, p)) out[key] = label;
  }
  return out;
}

/**
 * Display name for an author slug or a slot key.
 *
 * Order matters: the roster wins over everything, so a participant is named by
 * role and model even if its slug happens to collide with a reserved word.
 * An author with no roster row (a legacy row, a participant that has since
 * left) reads as {@link UNKNOWN_PARTICIPANT} — still attributed, never named
 * after an agent.
 */
export function authorLabel(
  author: string | null | undefined,
  labels: Record<string, string>,
): string {
  if (!author) return "";
  // `Object.hasOwn`, not a bare index: `labels["toString"]` answers out of
  // `Object.prototype`, and a FUNCTION is not null-ish, so `??` would not catch
  // it — a participant slugged `toString` would render a function body as its
  // byline. Every surface in the app resolves its author through here.
  if (Object.hasOwn(labels, author)) return labels[author];
  if (Object.hasOwn(NON_PARTICIPANT_AUTHORS, author))
    return NON_PARTICIPANT_AUTHORS[author];
  return UNKNOWN_PARTICIPANT;
}

/** Hook form of {@link participantLabelIndex}, memoised on the roster. */
export function useParticipantLabels(sessionId: string) {
  const { participants } = useSessionParticipants(sessionId);
  const labels = useMemo(() => participantLabelIndex(participants), [participants]);
  const hues = useMemo(() => participantHueIndex(participants), [participants]);
  return { participants, labels, hues };
}

/**
 * label → hue class, assigned by the participant's PLACE in the roster
 * (rc3 **D20**).
 *
 * **Rotation, not a hash**, and the difference is the whole point. Hashing a
 * label picks a hue that is stable and *probably* distinct; rotating over turn
 * slots makes distinctness a property of the assignment, so two participants in
 * one session cannot share a hue while the roster fits the palette.
 *
 * The shipped version hashed, on the reasoning that D20's ordinal already made
 * every label a distinct string. It did — and the palette held exactly TWO hues
 * against a roster of three, so a collision was not unlucky but certain. The
 * user reported it from a live session within the hour: *"HANDS and EYES-2 have
 * the same color."* Widening the palette to the roster cap fixes the certainty;
 * only rotation fixes the chance.
 *
 * Keyed by LABEL because that is what the render sites hold — the chat byline,
 * the turn-status line and the mention picker all resolve a participant to its
 * display string first, so the hue has to answer to the same key or one
 * participant gets two colours.
 */
export function participantHueIndex(
  participants: readonly ParticipantView[],
): Record<string, string> {
  const out: Record<string, string> = {};
  participants.forEach((p, i) => {
    // The user's pick wins; the rotation is the default, not a fallback for
    // failure. A name the palette no longer carries degrades to the rotation
    // rather than to no colour — an unknown entry costs the override, never the
    // participant.
    out[participantLabel(p)] = colorByName(p.color)?.token ?? participantHue(i);
  });
  return out;
}

// ---------------------------------------------------------------------------
// rc3 D11 — the capability warning
// ---------------------------------------------------------------------------

/**
 * Capability slug for editing files. Mirrors `Capability::EditFiles` in
 * `src/agents/capability.rs`; it reaches the dialog through
 * `RoleView.capabilities`, which `list_roles` already returns.
 */
export const EDIT_FILES = "edit_files";

/**
 * What the picked roster, taken together, cannot do.
 *
 * The user set the frame himself: *"how would bot-hq know EYES are reviewers?
 * Maybe warn that no participant can edit files (participant list has no write
 * capabilities ticked)."*
 *
 * So this reads TICKED BOXES and nothing else. It never looks at a role's name,
 * never counts roles, and never special-cases a duplicate — two of the same
 * role is simply one configuration that can produce this warning, exactly like
 * any other roster whose union is missing `edit_files`.
 *
 * ## Why this is not `CapabilitySet::warnings()`
 *
 * That Rust function answers a different question: it advises on ONE role's set
 * in isolation (self-review, read-only, silent worker). D11 asks what the UNION
 * of the picked roster cannot do — a different input and a different output, so
 * calling it would not answer this even with a command built to reach it. The
 * union is a set-membership test over `RoleView.capabilities`, which this
 * dialog has already loaded, so computing it here costs one pass over an array
 * and adds no command, no round-trip, and no second source of truth.
 *
 * Returns null when the roster is incomplete (a row with no role yet) — a
 * half-picked roster has not made a statement to warn about.
 */
export function capabilityGapWarning(
  picked: readonly { capabilities: readonly string[] }[],
): string | null {
  if (picked.length === 0) return null;
  const union = new Set<string>();
  for (const role of picked) for (const c of role.capabilities) union.add(c);
  if (union.has(EDIT_FILES)) return null;
  return "No participant can edit files — this session can review, but nothing in it can act.";
}

/**
 * Capability slug for filing review findings — the reviewer's box. Mirrors
 * `Capability::FileFinding` in `src/agents/capability.rs`.
 */
const FILE_FINDING = "file_finding";

/**
 * What the picked roster, taken together, should hear before Create (round 8).
 *
 * The D11 gap above stays the first word. Beyond it, two shapes a session got
 * created with in the wild and nothing pushed back on: a roster of two or more
 * with NO reviewer (a `hands` + `hands-2` session was created that way — the
 * only feedback a duplicate produced was the ordinal placeholder, which reads
 * as endorsement), and the same role picked more than once. Both are
 * advisory: duplicates are legitimate (two executors), and a solo roster is
 * the product default and is not told it lacks a reviewer.
 *
 * Returns null while the roster is incomplete or unremarkable.
 */
export function rosterAdvisory(
  picked: readonly { id: number; display_name: string; capabilities: readonly string[] }[],
): string | null {
  const gap = capabilityGapWarning(picked);
  if (gap) return gap;
  if (picked.length < 2) return null;
  const notes: string[] = [];
  const union = new Set<string>();
  for (const role of picked) for (const c of role.capabilities) union.add(c);
  if (!union.has(FILE_FINDING)) {
    notes.push(
      "No participant can file findings — nothing in this session can review the work.",
    );
  }
  const seen = new Map<number, string>();
  const dupes: string[] = [];
  for (const role of picked) {
    if (seen.has(role.id) && !dupes.includes(role.display_name)) dupes.push(role.display_name);
    seen.set(role.id, role.display_name);
  }
  if (dupes.length > 0) {
    notes.push(
      `${dupes.join(", ")} picked more than once — the second is ${dupes
        .map((d) => `${d}-2`)
        .join(", ")}; intended?`,
    );
  }
  return notes.length > 0 ? notes.join(" ") : null;
}
