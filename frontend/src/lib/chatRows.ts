import type { AgentMessage } from "./bindings";
import { describeToolCall, shortName } from "./runningTool";

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

/**
 * One tool call inside a burst, read once from its rows: the `tool_use` the
 * pump writes as `{input, name, tool_use_id}` and the `tool_result` it writes
 * as `{content, is_error, tool_use_id}`, whose `content` is the output text or,
 * for an MCP tool, a JSON string of content blocks (text, and images such as a
 * screenshot, base64).
 */
export interface ToolCall {
  /** The call's `tool_use_id`, or null when the row could not be read. */
  id: string | null;
  /** The tool without its MCP server prefix (`session_doc_write`). */
  tool: string;
  /** One human line: the call's own description, else the tool and its main argument. */
  label: string;
  /** The call's arguments, for the expanded view. */
  input: unknown;
  use: AgentMessage | null;
  result: AgentMessage | null;
  /** The result came back as an error (a failed command, a refused or blocked call). */
  failed: boolean;
  /** The result's text (every text block joined); "" while running or when there is none. */
  text: string;
  /** Images the result carries, as `data:` URLs — shown as thumbnails, never as base64. */
  images: string[];
}

/** One rendered chat row: a message as-is, or a run of consecutive passes. */
export type ChatRow =
  | { kind: "message"; key: number; message: AgentMessage }
  | { kind: "passes"; key: number; authors: string[]; createdAt: string };

/** A burst of one participant's consecutive tool calls (the visual pass, S3). */
export type BurstRow = { kind: "burst"; key: number; author: string; calls: ToolCall[]; firstAt: string; lastAt: string };

/** A row of the chat with its tool calls grouped into bursts. */
export type ChatRowWithBursts = ChatRow | BurstRow;

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

type Row = Record<string, unknown>;

function parseRow(content: string): Row {
  try {
    const value: unknown = JSON.parse(content);
    return value && typeof value === "object" ? (value as Row) : {};
  } catch {
    return {};
  }
}

/** A result's content as content blocks, when it is a list of them (an MCP
 *  tool's result is a JSON string of `[{type, …}]`); null for plain text. */
function blocksOf(raw: unknown): Row[] | null {
  let value = raw;
  if (typeof raw === "string") {
    if (!raw.startsWith("[{")) return null;
    try {
      value = JSON.parse(raw);
    } catch {
      return null;
    }
  }
  if (!Array.isArray(value) || value.length === 0) return null;
  const blocks = value.filter((b): b is Row => !!b && typeof b === "object");
  return blocks.length === value.length && blocks.every((b) => typeof b.type === "string") ? blocks : null;
}

interface ParsedResult {
  id: string | null;
  failed: boolean;
  text: string;
  images: string[];
}

// Rows are immutable and the store keeps their objects, so each is parsed once
// — a result can be hundreds of kilobytes (a screenshot, a long file).
const results = new WeakMap<AgentMessage, ParsedResult>();
const uses = new WeakMap<AgentMessage, ToolCall>();

function readResult(m: AgentMessage): ParsedResult {
  const kept = results.get(m);
  if (kept) return kept;
  const row = parseRow(m.content);
  const raw = row.content ?? row.output;
  const texts: string[] = [];
  const images: string[] = [];
  const blocks = blocksOf(raw);
  if (blocks) {
    for (const b of blocks) {
      if (b.type === "text" && typeof b.text === "string") texts.push(b.text);
      const source = b.source as Row | undefined;
      if (b.type === "image" && source && typeof source.data === "string") {
        const media = typeof source.media_type === "string" ? source.media_type : "image/png";
        images.push(`data:${media};base64,${source.data}`);
      }
    }
  } else if (typeof raw === "string") {
    texts.push(raw);
  } else if (raw !== undefined && raw !== null) {
    texts.push(JSON.stringify(raw));
  }
  const parsed = {
    id: typeof row.tool_use_id === "string" ? row.tool_use_id : null,
    failed: row.is_error === true,
    text: texts.join("\n"),
    images,
  };
  results.set(m, parsed);
  return parsed;
}

/** A fresh call for a `tool_use` row (the result fields are filled in when its
 *  result lands). Cached per row like the results, and copied out so a burst
 *  never mutates the cache. */
function readUse(m: AgentMessage): ToolCall {
  let kept = uses.get(m);
  if (!kept) {
    const row = parseRow(m.content);
    const name = typeof row.name === "string" ? row.name : "tool";
    const input = row.input ?? row.args ?? {};
    kept = {
      id: typeof row.tool_use_id === "string" ? row.tool_use_id : null,
      tool: shortName(name),
      label: describeToolCall(name, input),
      input,
      use: m,
      result: null,
      failed: false,
      text: "",
      images: [],
    };
    uses.set(m, kept);
  }
  return { ...kept };
}

function attach(call: ToolCall, m: AgentMessage, r: ParsedResult): void {
  call.result = m;
  call.failed = r.failed;
  call.text = r.text;
  call.images = r.images;
}

/**
 * The chat's rows with pass noise compacted (feedback #13): a `pass_turn` call
 * and its "pass noted" result are dropped, and consecutive pass lines — with
 * only those dropped rows between them — become ONE row naming who passed, in
 * order. Every other message is kept as-is and in place.
 */
export function compactRows(messages: readonly AgentMessage[]): ChatRow[] {
  return build(messages, false) as ChatRow[];
}

/**
 * `compactRows`, with **tool bursts** (the visual pass, S3): one participant's
 * consecutive tool rows — no text, notice or other participant between them —
 * become ONE row holding its calls, each paired with its result. A result that
 * lands after a notice still joins its own call; a notice is never folded into
 * a burst.
 */
export function chatRows(messages: readonly AgentMessage[]): ChatRowWithBursts[] {
  return build(messages, true);
}

function build(messages: readonly AgentMessage[], bursts: boolean): ChatRowWithBursts[] {
  const passCalls = new Set<string>();
  const rows: ChatRowWithBursts[] = [];
  // Calls still waiting for a result, by id, across bursts.
  const open = new Map<string, { call: ToolCall; burst: BurstRow }>();
  for (const m of messages) {
    const call = passCallId(m);
    if (call) {
      passCalls.add(call);
      continue;
    }
    const answered = resultCallId(m);
    if (answered && passCalls.has(answered)) continue;
    if (bursts && (m.kind === "tool_use" || m.kind === "tool_result")) {
      const last = rows[rows.length - 1];
      const current = last && last.kind === "burst" && last.author === m.author ? last : null;
      if (m.kind === "tool_result") {
        const r = readResult(m);
        const waiting = r.id ? open.get(r.id) : undefined;
        if (waiting && waiting.call.result === null) {
          attach(waiting.call, m, r);
          if (m.created_at > waiting.burst.lastAt) waiting.burst.lastAt = m.created_at;
          if (r.id) open.delete(r.id);
          continue;
        }
        // A result whose call is not on screen (an older page): its own call.
        const orphan: ToolCall = {
          id: r.id, tool: "result", label: "result", input: null, use: null,
          result: null, failed: false, text: "", images: [],
        };
        attach(orphan, m, r);
        if (current) {
          current.calls.push(orphan);
          current.lastAt = m.created_at;
        } else {
          rows.push({ kind: "burst", key: m.id, author: m.author, calls: [orphan], firstAt: m.created_at, lastAt: m.created_at });
        }
        continue;
      }
      const fresh = readUse(m);
      let burst = current;
      if (burst) {
        burst.calls.push(fresh);
        burst.lastAt = m.created_at;
      } else {
        burst = { kind: "burst", key: m.id, author: m.author, calls: [fresh], firstAt: m.created_at, lastAt: m.created_at };
        rows.push(burst);
      }
      if (fresh.id) open.set(fresh.id, { call: fresh, burst });
      continue;
    }
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

const MAX_TOOLS_NAMED = 4;

/** The tools a burst ran, in the order they first ran — `Bash ×3, Read ×2,
 *  session_doc_write` — then `+N more` past four names. First-run order, not
 *  most-used, so a growing burst's line does not reshuffle. */
export function burstTools(calls: readonly ToolCall[]): string {
  const counts = new Map<string, number>();
  for (const c of calls) counts.set(c.tool, (counts.get(c.tool) ?? 0) + 1);
  const named = [...counts.entries()].map(([tool, n]) => (n > 1 ? `${tool} ×${n}` : tool));
  const shown = named.slice(0, MAX_TOOLS_NAMED).join(", ");
  return named.length > MAX_TOOLS_NAMED ? `${shown}, +${named.length - MAX_TOOLS_NAMED} more` : shown;
}

/** How many of a burst's calls came back as errors. */
export function burstFailures(calls: readonly ToolCall[]): number {
  return calls.filter((c) => c.failed).length;
}

/** The first non-empty line of a result's text, clipped — its summary. */
export function firstLine(text: string, max = 140): string {
  const line = text.split("\n").find((l) => l.trim() !== "")?.trim() ?? "";
  return line.length > max ? `${line.slice(0, max - 1)}…` : line;
}

/** A burst's span: `14s`, `2m 05s`, `1h 03m`. */
export function formatSpan(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, "0")}s`;
  return `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, "0")}m`;
}
