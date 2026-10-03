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

// A tool row's content is JSON whose LAST key is `tool_use_id` (the pump
// writes `{input, name, tool_use_id}` and `{content, is_error, tool_use_id}`).
// The id is read from the tail without parsing the row: a `tool_result` holds
// a whole tool output, up to hundreds of kilobytes, and this runs on every
// render of the status line (EYES, apply review).
const ID_AT_END = /"tool_use_id"\s*:\s*"([^"\\]+)"\s*\}\s*$/;
const ID_AT_START = /^\s*\{\s*"tool_use_id"\s*:\s*"([^"\\]+)"/;
const SMALL_ENOUGH_TO_PARSE = 4096;

/** The `tool_use_id` of a tool row, or null when it cannot be read cheaply. */
function toolUseId(content: string): string | null {
  const fast = ID_AT_END.exec(content) ?? ID_AT_START.exec(content);
  if (fast) return fast[1];
  if (content.length > SMALL_ENOUGH_TO_PARSE) return null;
  try {
    const value: unknown = JSON.parse(content);
    const id = value && typeof value === "object" ? (value as Record<string, unknown>).tool_use_id : null;
    return typeof id === "string" ? id : null;
  } catch {
    return null;
  }
}

// The one row that IS parsed is the call in flight, for its label — and only
// once: rows are immutable, so the label is kept by row id.
const labels = new Map<number, string>();
const LABELS_KEPT = 200;

function labelOf(m: AgentMessage): string {
  const kept = labels.get(m.id);
  if (kept !== undefined) return kept;
  let label = "tool";
  try {
    const row = JSON.parse(m.content) as Record<string, unknown>;
    const name = typeof row.name === "string" ? row.name : "tool";
    label = describeToolCall(name, row.input ?? row.args);
  } catch {
    // An unreadable call still counts as running; it just has no better name.
  }
  if (labels.size >= LABELS_KEPT) labels.clear();
  labels.set(m.id, label);
  return label;
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
  const open: AgentMessage[] = [];
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m.kind === "system_notice" || m.kind === "phase_change") continue;
    if (m.author !== slug) break;
    if (m.kind !== "tool_use" && m.kind !== "tool_result") continue;
    const id = toolUseId(m.content);
    if (!id) continue;
    if (m.kind === "tool_result") answered.add(id);
    else if (!answered.has(id)) open.push(m);
  }
  if (open.length === 0) return null;
  const newest = open[0];
  return { label: labelOf(newest), startedAt: Date.parse(newest.created_at), others: open.length - 1 };
}
