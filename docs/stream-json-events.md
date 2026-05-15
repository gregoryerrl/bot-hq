# claude-code stream-json events

Empirical capture from `claude-code 2.1.141` on macOS. Raw samples live in
`docs/stream-json-samples/` (`plain.jsonl`, `tool_use.jsonl`, `malformed.jsonl`).
Re-run the probes whenever the CLI version changes — schema is informal and may
shift.

## Top-level shape

The output stream is **newline-delimited JSON**, one event per line. Every
object is tagged by a top-level `"type"` field (and most also carry
`session_id` + `uuid`). The events broadly mirror Anthropic's chat-message
shape: an `assistant` event wraps a single `message` whose `content[]` array
holds typed blocks (`text` / `thinking` / `tool_use`). Tool results are sent
**back into the conversation** as a `user` event whose `message.content[]` is
a `tool_result` block.

Important quirks observed:

- The CLI **does not echo our stdin** into the output stream. The first lines
  on stdout are always `system / hook_started` events for any `SessionStart`
  hooks (in our environment: 3 hooks → 3 `hook_started` + 3 `hook_response`).
- Each content block becomes its **own** `assistant` event line. Within the
  same Anthropic API turn, those events share a `message.id` (e.g. a
  `thinking` block and a `tool_use` block emitted as separate lines with the
  same `msg_...` id). A follow-up assistant turn after a tool result gets a
  **new** `message.id`.
- Per-event `message.stop_reason` is always `null` in our captures. The
  authoritative "agent turn finished" signal is the terminal **`result`**
  event (`stop_reason: "end_turn"`, `subtype: "success"|<error>`).
- `--input-format stream-json` requires `--verbose` when paired with
  `--print` + `--output-format stream-json`. Without it the CLI exits with
  `Error: When using --print, --output-format=stream-json requires --verbose`.
- Malformed stdin: the CLI emits the `SessionStart` hook events first (so we
  always see 6 `system` events), then writes `Error parsing streaming input
  line: ... SyntaxError: JSON Parse error: ...` to **stderr** and exits 1.
  **No `result` event is emitted on malformed input** — supervisor must treat
  non-zero exit + missing `result` as failure.

## Event types observed

| `type`              | `subtype`(s)                              | Source        | Notes                                                          |
| ------------------- | ----------------------------------------- | ------------- | -------------------------------------------------------------- |
| `system`            | `hook_started`, `hook_response`, `init`   | CLI lifecycle | Pre-turn metadata. `init` carries tools/model/mcp_servers/cwd. |
| `assistant`         | —                                         | Model turn    | One per content-block in the model's message.                  |
| `user`              | —                                         | Tool result   | `message.content[].type = "tool_result"` (Anthropic shape).    |
| `rate_limit_event`  | —                                         | CLI           | Periodic; carries `rate_limit_info` block.                     |
| `result`            | `success` (and presumably error variants) | CLI terminal  | **Last** event of a successful turn. Carries cost + final text.|

### Input shape (stdin)

`--input-format stream-json` expects newline-delimited JSON, **one user event
per line**, matching the shape we sent successfully:

```json
{"type":"user","message":{"role":"user","content":"say hello in 3 words"}}
```

This matches the same wire shape that `claude-code` itself uses when emitting
a `user` event for tool results — i.e. inputs ride the same envelope as the
conversation, just with `message.content` being a plain string (or an array of
content blocks for richer input).

## Per-event detail

### `system` — `hook_started` / `hook_response`

Pre-flight hook lifecycle. Bot-hq currently doesn't care about hook content
but should be ready to ignore (or surface as diagnostics) any number of these
before the first `assistant`/`result`.

```jsonc
// system / hook_started
{
  "type": "system",
  "subtype": "hook_started",
  "hook_id": "8f264d1e-...",
  "hook_name": "SessionStart:startup",
  "hook_event": "SessionStart",
  "uuid": "...",
  "session_id": "..."
}

// system / hook_response (success path)
{
  "type": "system",
  "subtype": "hook_response",
  "hook_id": "...",
  "hook_name": "SessionStart:startup",
  "hook_event": "SessionStart",
  "output": "<stringified hook stdout or JSON>",
  "stdout": "...",
  "stderr": "",
  "exit_code": 0,
  "outcome": "success",
  "uuid": "...",
  "session_id": "..."
}
```

### `system` — `init`

Emitted exactly once, immediately after the hook events. Carries the session
boot snapshot the supervisor may want to log/expose.

```jsonc
{
  "type": "system",
  "subtype": "init",
  "cwd": "/Users/.../bot-hq-rebuild",
  "session_id": "259f8ff5-21aa-40d5-8bf0-b772d89e4661",
  "model": "claude-opus-4-7[1m]",
  "permissionMode": "default",
  "tools": ["Task", "Bash", "Edit", "Read", "Write", "..."],
  "mcp_servers": [{ "name": "bot-hq", "status": "failed" }, "..."],
  "slash_commands": ["note", "investigate", "..."],
  "agents": ["..."],
  "plugins": ["..."],
  "skills": ["..."],
  "memory_paths": ["..."],
  "apiKeySource": "...",
  "claude_code_version": "2.1.141",
  "output_style": "default",
  "fast_mode_state": "off",
  "analytics_disabled": false,
  "uuid": "..."
}
```

### `assistant` — model content block

Emitted **once per content block** in the model's message. Multiple
consecutive `assistant` events sharing the same `message.id` belong to the
same turn. The interesting payload is `message.content[]` whose entries
discriminate by `type`:

- `text` → `{ "type":"text", "text":"..." }`
- `thinking` → `{ "type":"thinking", "thinking":"...", "signature":"..." }`
- `tool_use` → `{ "type":"tool_use", "id":"toolu_...", "name":"Bash", "input": {...}, "caller": { "type":"direct" } }`

```jsonc
// assistant text
{
  "type": "assistant",
  "message": {
    "id": "msg_01PmqiLmML5y9qcLjGQMDZUt",
    "model": "claude-opus-4-7",
    "type": "message",
    "role": "assistant",
    "content": [{ "type": "text", "text": "Hello there, friend." }],
    "stop_reason": null,
    "usage": { "input_tokens": 6, "output_tokens": 4, "...": "..." }
  },
  "parent_tool_use_id": null,
  "session_id": "...",
  "uuid": "..."
}

// assistant tool_use
{
  "type": "assistant",
  "message": {
    "id": "msg_0196WHHBCju5mbe4ca6St7f6",
    "content": [{
      "type": "tool_use",
      "id": "toolu_01MNwiN8DdLtUBVF1MLBvkmc",
      "name": "Bash",
      "input": { "command": "echo hello-from-claude", "description": "Print greeting message" },
      "caller": { "type": "direct" }
    }],
    "stop_reason": null,
    "...": "..."
  },
  "session_id": "...",
  "uuid": "..."
}
```

### `user` — tool result

Sent back into the conversation after the CLI executes a tool the assistant
called. `message.content[]` holds one `tool_result` block referencing the
`tool_use_id`. The CLI also surfaces a `tool_use_result` sibling at the
event top-level with raw stdout/stderr/exit metadata for the executed tool.

```jsonc
{
  "type": "user",
  "message": {
    "role": "user",
    "content": [{
      "tool_use_id": "toolu_01MNwiN8DdLtUBVF1MLBvkmc",
      "type": "tool_result",
      "content": "hello-from-claude",
      "is_error": false
    }]
  },
  "parent_tool_use_id": null,
  "session_id": "...",
  "uuid": "...",
  "timestamp": "2026-05-14T14:31:36.951Z",
  "tool_use_result": {
    "stdout": "hello-from-claude",
    "stderr": "",
    "interrupted": false,
    "isImage": false,
    "noOutputExpected": false
  }
}
```

### `rate_limit_event`

```jsonc
{
  "type": "rate_limit_event",
  "rate_limit_info": {
    "status": "allowed",
    "resetsAt": 1778782200,
    "rateLimitType": "five_hour",
    "overageStatus": "allowed",
    "overageResetsAt": 1778769000,
    "isUsingOverage": false
  },
  "uuid": "...",
  "session_id": "..."
}
```

### `result` — terminal turn signal

The **agent-turn-finished** event. Always last. Carries `subtype` (`success`
in our captures; assume error subtypes exist for API/permission failures),
`stop_reason`, the final stringified `result`, cost + token usage, and
`permission_denials`.

```jsonc
{
  "type": "result",
  "subtype": "success",
  "is_error": false,
  "api_error_status": null,
  "duration_ms": 3365,
  "duration_api_ms": 2948,
  "num_turns": 1,
  "result": "Hello there, friend.",
  "stop_reason": "end_turn",
  "session_id": "...",
  "total_cost_usd": 0.2725425,
  "usage": { "input_tokens": 6, "output_tokens": 12, "...": "..." },
  "modelUsage": { "claude-opus-4-7[1m]": { "...": "..." } },
  "permission_denials": [],
  "terminal_reason": "completed",
  "fast_mode_state": "off",
  "uuid": "..."
}
```

## Phase 2 must-handle checklist

The agent-subprocess wiring (Phase 2) must at minimum parse and react to:

1. **Assistant text** — `assistant` event whose `message.content[].type ==
   "text"` → append to chat transcript.
2. **Assistant tool_use** — `assistant` event with `message.content[].type ==
   "tool_use"` → record the call (id/name/input) so the matching `tool_result`
   can be paired.
3. **Tool result** — `user` event with `message.content[].type ==
   "tool_result"` → mark the tool call complete (success / error via
   `is_error`). The sibling `tool_use_result` field carries raw exec
   metadata if we want stdout/stderr separately.
4. **Turn-end signal** — `result` event (any subtype). This is the **only**
   reliable signal that the agent has finished; per-event `stop_reason`
   stays `null`. Use this as the duo-coordination handoff in A/V mode.
5. **Errors** — three failure surfaces:
   - `result` with `is_error: true` or non-`success` `subtype` /
     `api_error_status` populated.
   - Subprocess exit-code != 0 with **no** `result` event (e.g. malformed
     stdin → exit 1 + stderr parse error). Supervisor must treat
     missing-`result` as a hard failure.
   - `permission_denials` array on the `result` event when tools are
     blocked.

Nice-to-have but lower priority for Phase 2:

- `assistant` `thinking` blocks — keep raw bytes around (they include a
  `signature` field that must be passed back when resuming) but they need
  no UI rendering yet.
- `system / init` — log model + mcp_server statuses to detect MCP boot
  failures.
- `system / hook_*` — pass through / ignore. Surface only when a hook
  `exit_code != 0`.
- `rate_limit_event` — surface in status UI eventually.

## Rust struct sketch

Skeleton for `serde` parsing. `untagged` content-block enums are needed
inside `message.content[]` because tool_use input is arbitrary JSON.

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    System(SystemEvent),
    Assistant(AssistantEvent),
    User(UserEvent),
    RateLimitEvent(RateLimitEvent),
    Result(ResultEvent),
}

// ---- system -------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "subtype", rename_all = "snake_case")]
pub enum SystemEvent {
    HookStarted {
        hook_id: String,
        hook_name: String,
        hook_event: String,
        uuid: String,
        session_id: String,
    },
    HookResponse {
        hook_id: String,
        hook_name: String,
        hook_event: String,
        output: Option<String>,
        stdout: Option<String>,
        stderr: Option<String>,
        exit_code: Option<i32>,
        outcome: Option<String>,
        uuid: String,
        session_id: String,
    },
    Init {
        cwd: String,
        session_id: String,
        model: String,
        #[serde(rename = "permissionMode")]
        permission_mode: Option<String>,
        tools: Vec<String>,
        mcp_servers: Vec<McpServerStatus>,
        // Other fields stashed for forward-compat:
        #[serde(flatten)]
        extra: serde_json::Map<String, Value>,
    },
}

#[derive(Debug, Deserialize, Serialize)]
pub struct McpServerStatus { pub name: String, pub status: String }

// ---- assistant / user (Anthropic message envelope) ----------------------

#[derive(Debug, Deserialize, Serialize)]
pub struct AssistantEvent {
    pub message: AnthropicMessage,
    pub parent_tool_use_id: Option<String>,
    pub session_id: String,
    pub uuid: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UserEvent {
    pub message: AnthropicMessage,
    pub parent_tool_use_id: Option<String>,
    pub session_id: String,
    pub uuid: String,
    pub timestamp: Option<String>,
    pub tool_use_result: Option<ToolUseResultMeta>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AnthropicMessage {
    pub id: Option<String>,           // present for assistant, absent on tool_result user msgs
    pub role: String,                 // "assistant" | "user"
    #[serde(rename = "type", default)]
    pub message_type: Option<String>, // "message"
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub usage: Option<Value>,         // shape is rich; punt to Value for v1
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    Thinking { thinking: String, signature: String },
    ToolUse {
        id: String,            // toolu_...
        name: String,          // e.g. "Bash"
        input: Value,          // tool-specific
        caller: Option<Value>, // { "type": "direct" } in our captures
    },
    ToolResult {
        tool_use_id: String,
        content: Value,        // string OR array of content blocks
        #[serde(default)]
        is_error: bool,
    },
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ToolUseResultMeta {
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub interrupted: Option<bool>,
    #[serde(rename = "isImage")]
    pub is_image: Option<bool>,
    #[serde(rename = "noOutputExpected")]
    pub no_output_expected: Option<bool>,
}

// ---- result + rate limit ------------------------------------------------

#[derive(Debug, Deserialize, Serialize)]
pub struct ResultEvent {
    pub subtype: String,            // "success" | error variants TBD
    pub is_error: bool,
    pub api_error_status: Option<String>,
    pub duration_ms: u64,
    pub duration_api_ms: u64,
    pub num_turns: u32,
    pub result: Option<String>,
    pub stop_reason: Option<String>,
    pub session_id: String,
    pub total_cost_usd: Option<f64>,
    pub usage: Option<Value>,
    pub permission_denials: Vec<Value>,
    pub terminal_reason: Option<String>,
    pub uuid: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RateLimitEvent {
    pub rate_limit_info: Value,
    pub uuid: String,
    pub session_id: String,
}
```

## Stdin event shape (writing TO the CLI)

The supervisor writes newline-delimited JSON to the child's stdin. Confirmed
working envelope is the same one we use for tool results — a Claude
conversation `user` event:

```json
{"type":"user","message":{"role":"user","content":"prompt text or content-block array"}}
```

`content` accepts either a plain string (as in our probes) or a content-block
array (e.g. with `{"type":"tool_result", ...}` blocks) when feeding tool
output back in for an interactive session.

A simple Rust write-side type:

```rust
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputEvent {
    User { message: InputUserMessage },
}

#[derive(Debug, Serialize)]
pub struct InputUserMessage {
    pub role: &'static str, // "user"
    pub content: Value,     // string OR Vec<ContentBlock>
}
```
