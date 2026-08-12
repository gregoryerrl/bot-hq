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
 * Roster as a slug → display-label map, for surfaces that hold an author slug
 * rather than a participant row (chat messages, the Quickview, the busy line).
 */
export function labelsBySlug(
  participants: readonly ParticipantView[],
): Record<string, string> {
  const out: Record<string, string> = {};
  for (const p of participants) out[p.slug] = participantLabel(p);
  return out;
}

/**
 * Display name for a message/preview author.
 *
 * Order matters: the roster wins over everything, so a participant is named by
 * role and model even if its slug happens to collide with a reserved word.
 * An author with no roster row (a legacy row, an agent that has since left)
 * falls back to the slug rather than being dropped — the line still has to be
 * attributable.
 */
export function authorLabel(
  author: string | null | undefined,
  labels: Record<string, string>,
): string {
  if (!author) return "";
  return labels[author] ?? NON_PARTICIPANT_AUTHORS[author] ?? author;
}

/** Hook form of {@link labelsBySlug}, memoised on the roster. */
export function useParticipantLabels(sessionId: string) {
  const { participants } = useSessionParticipants(sessionId);
  const labels = useMemo(() => labelsBySlug(participants), [participants]);
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
