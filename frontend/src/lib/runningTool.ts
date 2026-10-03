import type { AgentMessage } from "./bindings";

// What a busy participant is running right now, for the composer's status
// line (feedback #79 / #91). The claude CLI used to nudge a model that had
// made several tool calls without text — "the user hasn't heard from you in a
// while" — and each answer was a chat line the user had to read past and the
// peers had to read at all. bot-hq now switches that nudge off at spawn; this
// is what tells the user the session is alive instead, from rows the UI
// already holds: no extra payload, nothing added to the chat.

/** The tool call a participant has in flight. */
export interface RunningTool {
  /** One line: the call's own `description`, else the tool and its main argument. */
  label: string;
  /** When the call's row was written (ms since the epoch). */
  startedAt: number;
  /** How many OTHER calls of the same run are still unanswered (parallel calls). */
  others: number;
}

const MAX_LABEL = 90;

function clip(text: string): string {
  const oneLine = text.replace(/\s+/g, " ").trim();
  return oneLine.length > MAX_LABEL ? `${oneLine.slice(0, MAX_LABEL - 1)}…` : oneLine;
}

/** `mcp__bot-hq-signaling__session_doc_write` → `session_doc_write`. */
function shortName(name: string): string {
  const cut = name.lastIndexOf("__");
  return name.startsWith("mcp__") && cut > 4 ? name.slice(cut + 2) : name;
}

// The argument that says what a call is about, in the order to look for it.
const MAIN_ARGS = ["file_path", "pattern", "query", "path", "slug", "url", "command", "skill", "prompt"];

/** One line for a tool call: its own `description` when it wrote one (Bash and
 *  Agent calls do, for the user), otherwise the tool's name and its main
 *  argument. A path is shown by its last two segments. */
export function describeToolCall(name: string, input: unknown): string {
  const args = input && typeof input === "object" ? (input as Record<string, unknown>) : {};
  const described = args.description;
  if (typeof described === "string" && described.trim() !== "") return clip(described);
  const tool = shortName(name);
  for (const key of MAIN_ARGS) {
    const value = args[key];
    if (typeof value === "string" && value.trim() !== "") {
      const shown = key === "file_path" || key === "path" ? value.split("/").slice(-2).join("/") : value;
      return clip(`${tool} ${shown}`);
    }
  }
  return clip(tool);
}

function parsed(content: string): Record<string, unknown> | null {
  try {
    const value: unknown = JSON.parse(content);
    return value && typeof value === "object" ? (value as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

/** The tool call `slug` has in flight: the newest `tool_use` row of its current
 *  run (its unbroken rows at the end of the chat — host notices do not break
 *  it, as for the turn's age) that no `tool_result` has answered yet. `null`
 *  when its last call was answered, when it is writing rather than running a
 *  tool, or when the rows cannot be read. */
export function runningTool(
  messages: readonly AgentMessage[] | undefined,
  slug: string,
): RunningTool | null {
  if (!messages) return null;
  const answered = new Set<string>();
  const open: { label: string; startedAt: number }[] = [];
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m.kind === "system_notice" || m.kind === "phase_change") continue;
    if (m.author !== slug) break;
    if (m.kind !== "tool_use" && m.kind !== "tool_result") continue;
    const row = parsed(m.content);
    const id = typeof row?.tool_use_id === "string" ? row.tool_use_id : null;
    if (!row || !id) continue;
    if (m.kind === "tool_result") {
      answered.add(id);
    } else if (!answered.has(id)) {
      const name = typeof row.name === "string" ? row.name : "tool";
      open.push({
        label: describeToolCall(name, row.input ?? row.args),
        startedAt: Date.parse(m.created_at),
      });
    }
  }
  if (open.length === 0) return null;
  return { ...open[0], others: open.length - 1 };
}
