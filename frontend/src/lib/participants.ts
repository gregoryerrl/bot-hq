import { useMemo } from "react";
import { useTauriQuery } from "../hooks/useInvoke";

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
 * ## THE ONE HAND-WRITTEN CONTRACT TYPE — reconcile HERE at merge
 *
 * `ParticipantView` and `list_session_participants` are being added by the
 * parallel BACKEND unit, which owns `frontend/src/lib/bindings.ts` and
 * regenerates it with `cargo run -- export-bindings`. This frontend unit must
 * not regenerate that file, so the shape below is transcribed by hand from the
 * shared contract.
 *
 * When the two units merge: delete `ParticipantView` from this file, import it
 * from `../lib/bindings` instead, and check the generated shape field-for-field
 * against the contract quoted here. Nothing else in the frontend declares it,
 * so this is the only place to look.
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
  /** `"active"` | `"observer"`. */
  participation_mode: string;
  enabled: boolean;
};

/** The read command the backend unit is adding. Named once, here. */
export const LIST_PARTICIPANTS_CMD = "list_session_participants";

/**
 * The contract's display rule, in one function:
 *
 * > `role_display_name · model_display_name`, e.g. `HANDS · Claude Opus 5`.
 * > When `role_display_name` is null fall back to the model alone; when both
 * > are null fall back to the slug.
 *
 * The slug fallback is the ONLY path that can put an internal key on screen,
 * and it only fires when there is nothing else to say.
 */
export function participantLabel(
  p: Pick<ParticipantView, "slug" | "role_display_name" | "model_display_name">,
): string {
  const role = p.role_display_name?.trim() || null;
  const model = p.model_display_name?.trim() || null;
  if (role && model) return `${role} · ${model}`;
  if (role) return role;
  if (model) return model;
  return p.slug;
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
export const UNKNOWN_PARTICIPANT = "Unknown participant";

/**
 * The key a SLOT-SHAPED runtime field lands under.
 *
 * Two of the backend's runtime payloads are still shaped as a fixed pair —
 * `SessionActivityEvent { brian_busy, rain_busy }` and
 * `SessionRuntime { brian_health, rain_health }`. Those field names are frozen
 * wire that names **turn slots, not agents**: `src/core/activity.rs` fills them
 * from `slugs.get(0)` / `slugs.get(1)`, and `src/tauri_cmd/sessions.rs` from
 * `handle.participants.get(0)` / `.get(1)`.
 *
 * The frontend used to unpack them under the literal keys `"brian"` / `"rain"`,
 * which no rc3 roster has — so every lookup keyed by a roster slug missed, the
 * mount backfill left every health dot blank, and the turn-status line printed
 * the raw key. A `#`-prefixed key cannot collide with a slug (slugs are
 * role-derived identifiers), which keeps the two spaces distinguishable in one
 * map instead of silently overwriting each other.
 */
export function slotKey(turnPosition: number): string {
  return `#slot${turnPosition}`;
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
 * A third participant has no slot key on the wire at all — the fixed pair
 * reports slots 0 and 1 only — so it resolves through its slug, which the live
 * events supply as soon as it acts.
 */
export function participantRuntimeKeys(
  p: Pick<ParticipantView, "slug" | "turn_position">,
): [string, string] {
  return [p.slug, slotKey(p.turn_position)];
}

/**
 * One participant's entry in a per-participant runtime map (health, context
 * occupancy, busy flags), looked up across both key spaces.
 *
 * `undefined` means "nothing reported for this participant", which every caller
 * already treats as unknown rather than empty.
 */
export function participantRuntime<T>(
  map: Record<string, T | undefined> | undefined,
  p: Pick<ParticipantView, "slug" | "turn_position">,
): T | undefined {
  if (!map) return undefined;
  for (const key of participantRuntimeKeys(p)) {
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
    for (const key of participantRuntimeKeys(p)) out[key] = label;
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
  return { participants, labels };
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
 * in isolation (self-review, observer, silent worker). D11 asks what the UNION
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
