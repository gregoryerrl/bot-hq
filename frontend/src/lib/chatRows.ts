import type { AgentMessage } from "./bindings";

/**
 * The row a pass posts — mirrors `PASS_NOTICE` in `src/core/pump.rs`. A pass
 * stays VISIBLE (rc3 D25: declining a turn is something the user can see);
 * what changes here is only how much of the chat it takes (feedback #13: one
 * pass was up to four rows — the agent's prose, the `pass_turn` call, its
 * "pass noted" result, and this line).
 */
export const PASS_NOTICE = "(passed — nothing to add this round)";

/** The tool name a pass is called by, with or without the MCP prefix. */
const PASS_TOOL = "pass_turn";

/** One rendered chat row: a message as-is, or a run of consecutive passes. */
export type ChatRow =
  | { kind: "message"; key: number; message: AgentMessage }
  | { kind: "passes"; key: number; authors: string[]; createdAt: string };

function isPassNotice(m: AgentMessage): boolean {
  return m.kind === "text" && m.content.trim() === PASS_NOTICE;
}

/** The `tool_use_id` of a `pass_turn` call row, or null. String checks come
 *  first so a long history is not JSON-parsed on every batch. */
function passCallId(m: AgentMessage): string | null {
  if (m.kind !== "tool_use" || !m.content.includes(PASS_TOOL)) return null;
  try {
    const parsed = JSON.parse(m.content) as { name?: unknown; tool_use_id?: unknown };
    const name = typeof parsed.name === "string" ? parsed.name : "";
    if (name !== PASS_TOOL && !name.endsWith(`__${PASS_TOOL}`)) return null;
    return typeof parsed.tool_use_id === "string" ? parsed.tool_use_id : null;
  } catch {
    return null;
  }
}

/** The `tool_use_id` a `tool_result` row answers, or null. */
function resultCallId(m: AgentMessage): string | null {
  if (m.kind !== "tool_result" || !m.content.includes("pass noted")) return null;
  try {
    const parsed = JSON.parse(m.content) as { tool_use_id?: unknown };
    return typeof parsed.tool_use_id === "string" ? parsed.tool_use_id : null;
  } catch {
    return null;
  }
}

/**
 * The chat's rows with pass noise compacted (feedback #13): a `pass_turn` call
 * and its "pass noted" result are dropped, and consecutive pass lines — with
 * only those dropped rows between them — become ONE row naming who passed, in
 * order. Every other message is kept as-is and in place.
 */
export function compactRows(messages: readonly AgentMessage[]): ChatRow[] {
  const passCalls = new Set<string>();
  const rows: ChatRow[] = [];
  for (const m of messages) {
    const call = passCallId(m);
    if (call) {
      passCalls.add(call);
      continue;
    }
    const answered = resultCallId(m);
    if (answered && passCalls.has(answered)) continue;
    if (isPassNotice(m)) {
      const last = rows[rows.length - 1];
      if (last && last.kind === "passes") {
        last.authors.push(m.author);
        last.createdAt = m.created_at;
      } else {
        rows.push({ kind: "passes", key: m.id, authors: [m.author], createdAt: m.created_at });
      }
      continue;
    }
    rows.push({ kind: "message", key: m.id, message: m });
  }
  return rows;
}
