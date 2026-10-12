import { createContext } from "react";

/** The state of one of THIS session's gated commands, keyed by the 8-hex id
 *  agents quote in chat (feedback #61): a status line that says "waiting on
 *  your approval of 236473a7" after you approved it is contradicted where it
 *  is read, by a chip beside the token. Only a token that RESOLVES to a gate
 *  of this session gets one (EYES P9 — short git shas share the shape). */
export type GateChip = {
  state: "pending" | "approved" | "rejected" | "withdrawn";
  /** The gate's full id, for the title. */
  choiceId: string;
};

/** The session's gate states by 8-hex prefix, from its tray rows: only rows
 *  that carry a command are gates; `answered` is approved when the pick is
 *  the literal Approve, rejected otherwise; the rest (withdrawn, superseded)
 *  read as withdrawn. A prefix two gates share is dropped — a chip must never
 *  claim a state that may belong to another gate. */
export function gateChipsOf(
  rows: readonly { choice_id: string; status: string; command_text: string | null; picked_option?: string | null }[],
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
    const state: GateChip["state"] =
      r.status === "pending"
        ? "pending"
        : r.status === "answered"
          ? r.picked_option === "Approve"
            ? "approved"
            : "rejected"
          : "withdrawn";
    out.set(key, { state, choiceId: r.choice_id });
  }
  for (const k of clashing) out.delete(k);
  return out;
}

/** Is `text` the shape of a gate id as quoted in chat — exactly 8 hex chars? */
export function looksLikeGateId(text: string): boolean {
  return /^[0-9a-f]{8}$/i.test(text.trim());
}

/** The chips of the session whose chat is being rendered; empty everywhere
 *  else (a Markdown outside a session shows no chips). */
export const GateChipsContext = createContext<ReadonlyMap<string, GateChip>>(new Map());
