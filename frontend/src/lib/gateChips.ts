import { createContext } from "react";

/** The state of one of THIS session's gated commands, keyed by the 8-hex id
 *  agents quote in chat (feedback #61): a status line that says "waiting on
 *  your approval of 236473a7" after you approved it is contradicted where it
 *  is read, by a chip beside the token. Only a token that RESOLVES to a gate
 *  of this session gets one (EYES P9 — short git shas share the shape). */
export type GateChip = {
  /** `approved` is an approval whose run has not ended (running, scheduled,
   *  or a row from before the outcome was recorded); `ran` ended with exit 0,
   *  `failed` with another code, `refused` never ran after the approval
   *  (feedback #104 — "approved" used to be shown over all of these). */
  state: "pending" | "approved" | "ran" | "failed" | "refused" | "rejected" | "withdrawn";
  /** What the chip shows: the state, or `exit N` for a failed run. */
  label: string;
  /** The gate's full id, for the title. */
  choiceId: string;
  /** Why an approved run was refused, for the title. */
  detail?: string;
};

/** The session's gate states by 8-hex prefix, from its tray rows: only rows
 *  that carry a command are gates; `answered` with the literal Approve reads
 *  as the RUN's outcome when one is recorded — `ran`, `exit N`, `refused` —
 *  and as approved until then; any other answer is rejected; the rest
 *  (withdrawn, superseded) read as withdrawn. A prefix two gates share is
 *  dropped — a chip must never claim a state that may belong to another gate. */
export function gateChipsOf(
  rows: readonly {
    choice_id: string;
    status: string;
    command_text: string | null;
    picked_option?: string | null;
    exit_code?: number | null;
    run_refusal?: string | null;
  }[],
): Map<string, GateChip> {
  const out = new Map<string, GateChip>();
  const clashing = new Set<string>();
  for (const r of rows) {
    if (!r.command_text) continue;
    const key = r.choice_id.slice(0, 8).toLowerCase();
    if (out.has(key)) {
      clashing.add(key);
      continue;
    }
    out.set(key, { ...outcomeOf(r), choiceId: r.choice_id });
  }
  for (const k of clashing) out.delete(k);
  return out;
}

function outcomeOf(r: {
  status: string;
  picked_option?: string | null;
  exit_code?: number | null;
  run_refusal?: string | null;
}): Pick<GateChip, "state" | "label" | "detail"> {
  if (r.status === "pending") return { state: "pending", label: "pending" };
  if (r.status !== "answered") return { state: "withdrawn", label: "withdrawn" };
  if (r.picked_option !== "Approve") return { state: "rejected", label: "rejected" };
  if (r.run_refusal) return { state: "refused", label: "refused", detail: r.run_refusal };
  if (r.exit_code === null || r.exit_code === undefined) return { state: "approved", label: "approved" };
  if (r.exit_code === 0) return { state: "ran", label: "ran" };
  return { state: "failed", label: r.exit_code === 124 ? "timed out" : `exit ${r.exit_code}` };
}

/** Is `text` the shape of a gate id as quoted in chat — exactly 8 hex chars? */
export function looksLikeGateId(text: string): boolean {
  return /^[0-9a-f]{8}$/i.test(text.trim());
}

/** The chips of the session whose chat is being rendered; empty everywhere
 *  else (a Markdown outside a session shows no chips). */
export const GateChipsContext = createContext<ReadonlyMap<string, GateChip>>(new Map());
