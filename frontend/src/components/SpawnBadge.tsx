import { cn } from "../lib/cn";
import type { ParticipantView } from "../lib/participants";

/**
 * What a participant was actually spawned with — its effort posture, or
 * ultracode.
 *
 * ## Why this exists, and why it reads a different field than you'd expect
 *
 * `tauri_cmd/sessions.rs` states the gap: *"the New Session dialog writes both
 * this and `ultracode` per row and nothing could read them back, so the session
 * view had no way to show what a running participant was actually spawned
 * with."*
 *
 * The obvious fix — render `p.effort` — does not close it. That field is the
 * user's **choice**, and Default (the absence pair) is the common choice, so
 * rendering it would say nothing about most participants. The effective value
 * comes from the chain (per-run pick → the role's default → the
 * `DEFAULT_EFFORT` floor, since no-inherit 2026-08-25) plus a reconciliation
 * that can flip either knob, because claude-code treats `effort=max` and
 * `ultracode` as mutually exclusive.
 *
 * The frontend **cannot** compute that: `claude-overrides.json` keys its scopes
 * by ROLE SLUG and `ParticipantView` carries no role slug. Re-deriving it in TS
 * would put the precedence chain in a second language behind a
 * stay-in-step comment — the guard that already failed once this round — and
 * unlike a wrong display name, a wrong answer here is a confident technical
 * claim on a badge whose entire justification is accuracy.
 *
 * So the backend records the reconciled pair at spawn (migration 0061), the
 * same call `slot0_model_at_spawn` made for the sibling fact, and this renders
 * it. Re-resolving at read time would answer a different question — *"what it
 * would be spawned with now"* — which diverges the moment Claude Config is
 * edited mid-session.
 *
 * ## The three states, and why silence is only one of them
 *
 * | row | renders |
 * |---|---|
 * | `spawn_knobs_recorded === false` | **nothing** — spawned before 0061; unknown, and a guess is worse than a gap |
 * | recorded, both null | `default` — a pre-floor row: spawned before 2026-08-25, when reconciling to nothing was possible |
 * | recorded, either set | the value |
 *
 * The flag is what separates rows 1 and 2, which are otherwise the same two
 * nulls. Every post-floor spawn records a concrete effort, so row 2 only
 * shrinks from here — but it stays renderable for the rows that exist.
 *
 * ## Text says what, style says whether it was chosen
 *
 * The participant's own `effort` / `ultracode` are consulted **only** for
 * styling: a value this run explicitly picked renders emphasised, an inherited
 * one muted. That way one badge answers both questions without claiming the
 * second in words it cannot support.
 */
export function SpawnBadge({
  participant: p,
}: {
  participant: Pick<
    ParticipantView,
    | "effort"
    | "ultracode"
    | "effort_at_spawn"
    | "ultracode_at_spawn"
    | "spawn_knobs_recorded"
  >;
}) {
  // Nothing was recorded for this row, so nothing is known. Rendering "default"
  // here would assert a configuration this participant may not have run with.
  if (!p.spawn_knobs_recorded) return null;

  const effort = p.effort_at_spawn?.trim() || null;
  const ultracode = p.ultracode_at_spawn === true;
  // Ultracode wins the label because the reconciliation already made them
  // mutually exclusive — if both arrived, the row is telling us something the
  // backend promised cannot happen, and showing the stronger posture is the
  // safer of the two readings.
  const text = ultracode ? "ultracode" : (effort ?? "default");

  // Chosen for THIS run, rather than resolved from the role's default. Read off
  // the choice columns, never off the effective pair: a role-default `high` and
  // a picked `high` are the same string and a different fact.
  const chosen = ultracode ? p.ultracode !== null : p.effort !== null;

  return (
    <span
      className={cn(
        "ml-1 rounded border px-1.5 py-0.5 font-label-caps text-label-caps",
        chosen
          ? "border-primary/50 bg-primary/15 text-primary"
          : "border-outline-variant text-on-surface-variant",
      )}
      title={
        chosen
          ? `Spawned with ${text} — picked for this session.`
          : `Spawned with ${text} — the role's default.`
      }
    >
      {text}
    </span>
  );
}
