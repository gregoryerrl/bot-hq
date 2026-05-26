# Tauri v2 Migration Implementation Plan

> **Execution:** Use the `superpowers:executing-plans` skill to implement this plan task-by-task.

**Goal:** Migrate bot-hq's UI shell from Slint to Tauri v2 + React + shadcn/ui in a worktree off main (`../bot-hq-tauri`, branch `tauri-v2-migration`), preserving the Rust core verbatim through every commit.

**Architecture:** Big-bang in an isolated worktree. Single `SignalingBridge` as SSOT; two dispatch surfaces (existing internal/external HTTP MCP for agents, new Tauri commands for frontend+plugins). Streaming via Tauri events with `BatchEmitter` coalescing the hot path via the existing `messages_for_session(session_id, since_id)` query (event-triggered batch fetch — see "Batch 1 correction" below). Plugins as iframes with per-plugin custom URI scheme + capability JSON.

**Tech Stack:** Rust (existing core, zero LOC delta in `src/agents`, `src/core`, `src/policy`, `src/storage`, `src/signaling`) + Tauri v2 (`tauri`, `tauri-build`, `tauri-specta`, `specta`) + React 18 + TypeScript + Tailwind + shadcn/ui (Vite build) + Zustand + TanStack Query + Vitest + React Testing Library.

**Branch:** `tauri-v2-migration` in worktree `/Users/gregoryerrl/Projects/bot-hq-tauri`. Push grant: session-level, scope=specific, branches=["tauri-v2-migration"]. No `--no-verify`, no AI co-author trailers, imperative ≤72-char commit subjects.

**Regression baseline:** 206 Rust tests must stay green through every commit (158 lib + 31 external_mcp + 7 signaling + 10 storage). `cargo test`, `cargo clippy`, `cd frontend && pnpm test`, `tsc --noEmit` gate every commit.

**Force-flush on turn-end:** Path A (zero-delta, ≤50ms tail latency on turn-end). Path B (`SignalingEvent::TurnEnded` variant + per-agent-pump fire) deferred. Revisit only if profiling shows perceived lag.

---

## Numbers (verified against live codebase 2026-05-26)

| Metric | Live | Design doc | Delta |
|--------|------|-----------|-------|
| Rust tests | 206 | 202 | +4 since doc |
| Slint LOC to delete | 7,560 | 6,700 | +860 |
| `src/ui/mod.rs` | 9 LOC | not listed | add to deletion scope |
| `src/ui/view_model.rs` | 3,379 LOC | 2,840 LOC | +539 |
| `ui/app.slint` | 4,172 LOC | 3,846 LOC | +326 |

React frontend LOC estimate: **4,000–6,000 LOC**. Net delta: closer to flat than design doc's "-2,500 to +500".

---

## Batch 1 correction (event-triggered batch fetch via since_id watermark)

**Trigger discovered during planning:** `SignalingEvent::MessagePersisted { session_id, message_id }` carries IDs only — no content payload. The `src/signaling/bridge.rs` core is zero-delta. Bridge cannot be modified to push content.

**Resolution:** BatchEmitter tracks per-session `since_id` watermarks. On flush (N=20 or 50ms timer), it calls the existing `storage::messages_for_session(session_id, since_id=last_emitted)` to fetch all new messages in one indexed SELECT, then emits via Tauri.

**Verified surfaces:**

- `src/signaling/bridge.rs:68` — `MessagePersisted { session_id: String, message_id: i64 }`
- `src/signaling/bridge.rs:440` — `pub fn subscribe() -> broadcast::Receiver<SignalingEvent>`
- `src/signaling/bridge.rs:834` — `pub fn notify_message_persisted(...)` (existing emit site)
- `src/storage/mod.rs:200` — `pub async fn messages_for_session(session_id, since_id: Option<i64>)`
- `src/signaling/external_jsonrpc.rs:279` — external MCP's `wait_for_change` already uses this subscribe + fetch-with-since_id pattern

Frontend-facing Tauri event shape is **identical** to the design doc — `agent.messages.batch` with `Vec<AgentMessage>` payload. The interface change is internal to the events layer.

---

## Batch sequence (8 batches, each one logical commit)

### Batch 0 — Foundation & smoke-tests
### Batch 1 — Tauri events layer (BatchEmitter + bridge subscriber)
### Batch 2 — Tauri commands layer (domain-grouped wrappers)
### Batch 3 — Plugin module scaffolding
### Batch 4 — main.rs Tauri bootstrap (cut-over point)
### Batch 5 — React frontend (may split into 5a/5b/5c if >3,000 LOC)
### Batch 6 — Slint removal (the deletion commit)
### Batch 7 — PROGRESS.md + docs + manual smoke gate

Each batch lands as one commit. After Batch 6, all 206 existing tests + new Tauri tests must be green, release build clean, Playwright smoke E2E green.

---

## Batch 0 — Foundation & smoke-tests

**Goal:** De-risk `tauri-specta` + Tauri v2 + Vite + shadcn/ui pipeline before any wrappers depend on it. Zero core delta; Slint binary still builds.

**Files:**
- Create: `Cargo.toml` (modify — add tauri deps), `build.rs` (modify), `src-tauri/tauri.conf.json`, `src-tauri/capabilities/main.json`, `src/tauri_specta_gen.rs`, `frontend/package.json`, `frontend/tsconfig.json`, `frontend/vite.config.ts`, `frontend/tailwind.config.ts`, `frontend/postcss.config.js`, `frontend/index.html`, `frontend/src/main.tsx`, `frontend/src/index.css`, `frontend/src/App.tsx`, `.gitignore` (extend).

**Step 0.1 — Commit this plan as the first artifact**

```bash
git add docs/plans/2026-05-26-tauri-v2-migration-implementation.md
# Call check_commit_message MCP tool with proposed message before committing
git commit -m "docs: add Tauri v2 migration implementation plan"
```

**Step 0.2 — Add Tauri Rust deps to `Cargo.toml`**

Add under `[dependencies]`:
```toml
tauri = { version = "2", features = ["macos-private-api"] }
specta = "2"
tauri-specta = { version = "2", features = ["typescript"] }
```
Add under `[build-dependencies]`:
```toml
tauri-build = "2"
```
Keep all existing deps (Slint stays linked through Batch 5).

**Step 0.3 — Modify `build.rs`**

```rust
fn main() {
    slint_build::compile("ui/app.slint").expect("Slint compile failed");
    tauri_build::build();
}
```

**Step 0.4 — Create `src-tauri/tauri.conf.json`**

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "bot-hq",
  "version": "0.1.0",
  "identifier": "com.gregoryerrl.bot-hq",
  "build": {
    "frontendDist": "../frontend/dist",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "cd frontend && pnpm build",
    "beforeDevCommand": "cd frontend && pnpm dev"
  },
  "app": {
    "windows": [{ "title": "bot-hq", "width": 1400, "height": 900 }],
    "security": { "csp": null }
  }
}
```

**Step 0.5 — Create `src-tauri/capabilities/main.json`**

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "default capability set",
  "windows": ["main"],
  "permissions": ["core:default"]
}
```

**Step 0.6 — Create `src/tauri_specta_gen.rs`**

```rust
use tauri_specta::{collect_commands, Builder};

pub fn builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_constructs_with_empty_commands() {
        let _b = builder();
    }

    #[test]
    fn builder_exports_to_typescript_stub() {
        let b = builder();
        b.export(
            specta_typescript::Typescript::default(),
            "/tmp/bot-hq-types-smoke.ts",
        ).expect("tauri-specta export must succeed for empty command set");
    }
}
```

Wire from `src/lib.rs`: add `pub mod tauri_specta_gen;`.

**Step 0.7 — Run Rust smoke test**

```bash
cargo test tauri_specta_gen -- --nocapture
```
Expected: 2 tests pass.

**Step 0.8 — Bootstrap frontend workspace**

Create `frontend/package.json`:
```json
{
  "name": "bot-hq-frontend",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "test": "vitest run",
    "lint": "tsc --noEmit"
  },
  "dependencies": {
    "@tauri-apps/api": "^2.0.0",
    "react": "^18.3.1",
    "react-dom": "^18.3.1",
    "zustand": "^5.0.0",
    "@tanstack/react-query": "^5.59.0"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2.0.0",
    "@vitejs/plugin-react": "^4.3.2",
    "@testing-library/react": "^16.0.1",
    "@testing-library/jest-dom": "^6.5.0",
    "@types/react": "^18.3.11",
    "@types/react-dom": "^18.3.0",
    "autoprefixer": "^10.4.20",
    "jsdom": "^25.0.1",
    "postcss": "^8.4.47",
    "tailwindcss": "^3.4.13",
    "typescript": "^5.6.2",
    "vite": "^5.4.8",
    "vitest": "^2.1.2"
  }
}
```

**Step 0.9 — Frontend smoke test**

`frontend/src/App.test.tsx`:
```typescript
import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import App from "./App";

describe("App", () => {
  it("renders the bot-hq shell", () => {
    render(<App />);
    expect(screen.getByText(/bot-hq/i)).toBeInTheDocument();
  });
});
```

`frontend/src/App.tsx`:
```tsx
export default function App() {
  return <div className="p-8 text-2xl">bot-hq</div>;
}
```

```bash
cd frontend && pnpm install && pnpm test
```
Expected: 1 test pass.

**Step 0.10 — Verify Slint binary still builds**

```bash
cargo build --release
```
Expected: clean.

**Step 0.11 — Verify all 206 existing tests still pass**

```bash
cargo test 2>&1 | grep -E "^test result"
```
Expected: 158 + 31 + 7 + 10 + (2 new tauri_specta_gen) = 208 passed, 0 failed.

**Step 0.12 — Commit Batch 0**

```bash
# Call check_commit_message MCP tool first
git add Cargo.toml Cargo.lock build.rs src-tauri/ src/tauri_specta_gen.rs src/lib.rs frontend/ .gitignore
git commit -m "build: scaffold Tauri v2 + Vite + shadcn/ui foundation"
```

**Step 0.13 — Push Batch 0** (session grant active)

```bash
git push -u origin tauri-v2-migration
```

---

## Batch 1 — Tauri events layer

**Goal:** Typed event structs + BatchEmitter (since_id watermark pattern) + bridge subscriber routing.

**Files:**
- Create: `src/tauri_events/mod.rs`, `src/tauri_events/types.rs`, `src/tauri_events/batch_emitter.rs`, `src/tauri_events/bridge_subscriber.rs`, `src/tauri_events/tests.rs`.
- Modify: `src/lib.rs` (add `pub mod tauri_events;`).

**Step 1.1 — Write failing test for AgentMessage event shape**

`src/tauri_events/tests.rs`:
```rust
#[test]
fn agent_message_has_event_name() {
    assert_eq!(AgentMessage::EVENT_NAME_BATCH, "agent.messages.batch");
}

#[test]
fn agent_message_serializes_to_expected_shape() {
    let msg = AgentMessage {
        session_id: "s1".to_string(),
        agent: "brian".to_string(),
        text: "hello".to_string(),
        created_at: 1234567890,
    };
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["session_id"], "s1");
    assert_eq!(json["agent"], "brian");
}
```

**Step 1.2 — Run test, verify FAIL** (compile error expected).

**Step 1.3 — Implement `src/tauri_events/types.rs`**

```rust
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AgentMessage {
    pub session_id: String,
    pub agent: String,
    pub text: String,
    pub created_at: i64,
}

impl AgentMessage {
    pub const EVENT_NAME_BATCH: &'static str = "agent.messages.batch";
}

// Repeat for SessionPhaseChanged, ClRefreshed, McpToolCalled, SessionCreated, SessionSubprocessDied
```

**Step 1.4 — Run tests, verify PASS**

**Step 1.5 — Write failing test for BatchEmitter (since_id watermark)**

```rust
#[tokio::test(start_paused = true)]
async fn batch_emitter_fetches_with_watermark() {
    let storage = test_storage_with([
        msg(1, "s1", "brian", "hello"),
        msg(2, "s1", "brian", "world"),
    ]);
    let captured = Arc::new(Mutex::new(Vec::new()));
    let cap_clone = captured.clone();
    let emitter = BatchEmitter::new(
        move |msgs: Vec<AgentMessage>| cap_clone.lock().unwrap().push(msgs),
        storage.clone(),
    );

    emitter.touch("s1".into(), 1).await;
    emitter.touch("s1".into(), 2).await;
    tokio::time::advance(Duration::from_millis(51)).await;
    tokio::task::yield_now().await;

    let captured = captured.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].len(), 2);
}

#[tokio::test(start_paused = true)]
async fn batch_emitter_watermark_advances_across_flushes() {
    // First flush 1-3 → watermark = 3. Second flush 4-5 → only new.
}

#[tokio::test(start_paused = true)]
async fn batch_emitter_coalesces_at_n_20() {
    // 20 touches → one query (not 20).
}
```

**Step 1.6 — Implement `BatchEmitter` and `bridge_subscriber`**

See plan-batch-1-correction (session doc, archived) for the complete sketch. Key points:
- Owns `tokio::mpsc::UnboundedReceiver<EmitMsg>` + `HashMap<String, i64>` watermarks + dirty set.
- `Touch` enqueues; `Flush` triggers per-session fetch via `storage.messages_for_session(sid, since_id)`.
- Bridge subscriber routes `SignalingEvent` variants:
  - `MessagePersisted` → `emitter.touch(session_id, message_id)`
  - `PendingChoice`, `AwaitingUser`, `ChoiceResolved`, `AgentAdvancePhase` → direct `app.emit(...)`

**Step 1.7 — All tauri_events tests green; full suite still 206+ pass**

**Step 1.8 — Commit + push Batch 1**

```bash
git add src/tauri_events/ src/lib.rs
git commit -m "feat: add Tauri events layer with BatchEmitter"
git push
```

---

## Batch 2 — Tauri commands layer

**Goal:** Domain-grouped `#[tauri::command]` wrappers calling `SignalingBridge`. Each command has a unit test using `tauri::test::mock_builder()` for AppHandle-requiring commands, direct bridge calls otherwise.

**Files:** `src/tauri_cmd/{mod.rs,error.rs,sessions.rs,agents.rs,mcp_tools.rs,settings.rs,policy.rs,cl.rs,plugins.rs,tests.rs}`.

**Step 2.1 — Write failing test for `create_session`**:

```rust
#[tokio::test]
async fn create_session_returns_session_info() {
    let (bridge, _data) = test_bridge().await;
    let info = sessions::create_session_inner(&bridge, "test session", None, "bot-hq").await.unwrap();
    assert!(!info.id.is_empty());
}
```

**Step 2.2 — Implement `src/tauri_cmd/error.rs` + `src/tauri_cmd/sessions.rs`**

```rust
// error.rs
#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "type")]
pub enum AppError {
    Validation(String),
    NotFound(String),
    Unauthorized(String),
    Internal(String),
    DbError(String),
    CapabilityDenied(String),
}

// sessions.rs
#[tauri::command]
#[specta::specta]
pub async fn create_session(
    state: tauri::State<'_, Arc<SignalingBridge>>,
    title: String,
    repo_path: Option<String>,
    project: String,
) -> Result<SessionInfo, AppError> {
    create_session_inner(&state, &title, repo_path.as_deref(), &project).await
}
```

**Step 2.3 — Wire commands into `tauri_specta_gen::builder()`**

```rust
.commands(collect_commands![
    sessions::create_session,
    sessions::respawn_session,
    sessions::close_session,
    sessions::get_session,
    sessions::list_sessions,
])
```

**Step 2.4 — Verify tauri-specta export updates frontend types**

```bash
cargo test tauri_specta_gen
cd frontend && pnpm exec tsc --noEmit
```

**Step 2.5 — Repeat 2.1-2.4 per domain group** (agents, mcp_tools, settings, policy, cl, plugins).

**Step 2.6 — Commit + push Batch 2**

```bash
git commit -m "feat: add Tauri command layer over SignalingBridge"
git push
```

---

## Batch 3 — Plugin module scaffolding

**Goal:** `src/plugins/` with manifest parser + loader + capability JSON generator + heartbeat watcher. Skeleton only — no live plugins.

**Files:** `src/plugins/{mod.rs,manifest.rs,loader.rs,capabilities.rs,heartbeat.rs,iframe_ipc_test.rs}`.

**Step 3.1 — TDD `PluginManifest::parse`** (pure-function tests).

**Step 3.2 — TDD `CapabilityGen::for_plugin`** — assert per-plugin allowed-commands list matches `requested_capabilities`.

**Step 3.3 — Dummy iframe origin test** (Rain's coverage gap from design doc).

**Step 3.4 — Heartbeat module skeleton** (app-shell scope, not per-PluginSlot).

**Step 3.5 — Commit + push Batch 3**

```bash
git commit -m "feat: scaffold plugin module with capability gating"
git push
```

---

## Batch 4 — main.rs Tauri bootstrap (cut-over point)

**Goal:** Replace Slint event loop in `main.rs` with Tauri builder. Tauri owns OS main thread; Tokio multi-thread on workers (unchanged). Existing `reap_all_children` + `CHILD_PIDS` + panic-hook + SIGTERM/SIGINT/SIGHUP signal task preserved verbatim.

**Step 4.1 — Restructure `main.rs`**:

```rust
fn main() {
    crash_protection::install_panic_hook();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
    let bridge = rt.block_on(async { /* existing SignalingBridge::new(...) */ });

    tauri::Builder::default()
        .manage(Arc::new(bridge))
        .invoke_handler(tauri_specta_gen::builder().build())
        .setup(|app| {
            // Spawn internal + external MCP HTTP servers
            // Spawn signal-task (SIGTERM/SIGINT/SIGHUP → reap_all_children → exit)
            // Spawn bridge subscriber for Tauri events
            // Auto-spawn Emma
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tauri run failed");
}
```

**Step 4.2 — Run all 206+ tests; verify clean.**

**Step 4.3 — Manual smoke:** `cargo run --release` → empty Tauri webview; lock file held; MCP ports bound.

**Step 4.4 — Commit + push Batch 4**

```bash
git commit -m "feat: bootstrap Tauri runtime in main.rs"
git push
```

---

## Batch 5 — React frontend

**Goal:** Port full Slint UI to React. Split into 5a/5b/5c if diff exceeds ~3,000 LOC by end of 5B.

**Files (`frontend/src/`):**
- App shell: `main.tsx`, `App.tsx`, `Router.tsx`, `Providers.tsx`
- Routes: `app/Dashboard.tsx`, `app/SessionView.tsx`, `app/Settings.tsx`, `app/ContextLibrary.tsx`, `app/PluginManager.tsx`
- shadcn primitives + composed: `components/ui/`, `components/{PhasePill,SessionTile,ChatInput,DocumentPane,EmmaOverlay,PluginSlot}.tsx`
- Features: `features/sessions/`, `features/chat/`, `features/cl/`, `features/docs/`, `features/policy/`, `features/plugins/`
- Hooks: `hooks/{useInvoke,useTauriEvent,useSession,useDocs,useCl,useChat}.ts`
- Stores (Zustand): `stores/{layout,emma,chat}.ts`

**Step 5.1 — Failing test for `useInvoke`** (mocks via `@tauri-apps/api/mocks`).

**Step 5.2 — Implement `useInvoke`** — TanStack Query mutation/query factory.

**Step 5.3 — Per-feature TDD** — Dashboard, SessionView (chat + DocumentPane 60/40 split), Settings, CL, PluginManager.

**Step 5.4 — Live git-diff rendering in A tab** via `tauri_cmd::docs::compute_apply_diff` (port Rust-side `parse_diff_lines` from Slint era — keeps GitHub-style coloring single-sourced).

**Step 5.5 — Author color coding** matches Slint baseline (brian=orange, rain=purple, emma=green, user=blue, system=muted grey).

**Step 5.6 — Vitest coverage ≥70% for hooks + stores**.

**Step 5.7 — Manual smoke checklist:**
- `+ New session` works; agent streams visible
- Phase pills act as DocumentPane tab selectors (NOT phase-advancers)
- Emma overlay slides in/out
- Live git-diff renders in A tab after commit
- `ask_user_choice` choice buttons appear inline + in banner

**Step 5.8 — Commit + push Batch 5**

```bash
git commit -m "feat: implement React frontend over Tauri IPC"
git push
```

---

## Batch 6 — Slint removal

**Goal:** Delete `src/ui/`, `ui/`, drop Slint deps, update docs.

**Files:**
- Delete: `src/ui/`, `ui/`.
- Modify: `Cargo.toml` (drop `slint`, `slint-build`), `build.rs` (drop `slint_build::compile`), `src/lib.rs` (drop `pub mod ui;`).
- Modify: `ARCHITECTURE.md`, `CLAUDE.md`, CL `conventions.md` + `notes.md`.

**Step 6.1 — Pre-deletion test sweep** — `cargo test`, all green.

**Step 6.2 — Delete UI** — `git rm -r src/ui/ ui/`.

**Step 6.3 — Drop Slint deps from `Cargo.toml`**.

**Step 6.4 — Simplify `build.rs`** — only `tauri_build::build()`.

**Step 6.5 — Drop `pub mod ui;` from `src/lib.rs`**.

**Step 6.6 — `cargo check` + `cargo test`** — clean, all green.

**Step 6.7 — Update docs in-repo + CL** (`conventions.md`, `notes.md`, `ARCHITECTURE.md`, `CLAUDE.md` — drop Slint sections + slint-rust-docs cross-references).

**Step 6.8 — Commit + push Batch 6**

```bash
git commit -m "remove: drop Slint UI and slint-rust-docs references"
git push
```

---

## Batch 7 — PROGRESS.md + polish

**Step 7.1 — PROGRESS.md newest-first entry** summarizing batches 0-6.

**Step 7.2 — Commit + push Batch 7**

```bash
git commit -m "docs: log Tauri v2 migration in PROGRESS.md"
git push
```

---

## CI gates (every commit)

```bash
# Rust side
cargo test 2>&1 | grep -E "FAILED|test result"
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release

# Frontend side (after Batch 0)
cd frontend && pnpm test && pnpm exec tsc --noEmit && pnpm build
```

No green = no commit. Do NOT amend or rebase landed batches.

---

## Push discipline

Session-level push grant active: `action=push`, `scope=specific`, `branches=["tauri-v2-migration"]`. Pushes execute without per-action approval prompts for the duration of this session.

- DO NOT push to `main` from this branch (no grant for main).
- DO NOT force-push (blocked by `policy.yaml` `tool_blocklist` + needs per-action approval anyway).
- If a batch needs revision: new commit, not `--amend`.

---

## Risk register

1. **`tauri-specta` + Tauri v2 incompatibility** — Batch 0 smoke-tests before downstream depends. Fallback: hand-rolled IPC type definitions (~500 LOC).
2. **React frontend LOC overrun** — split Batch 5 into 5a/5b/5c if >3,000 LOC.
3. **Subprocess reaper regression** — Batch 4 preserves verbatim; new test verifies SIGTERM → reaper fires.
4. **CSP + iframe origin denial** — Batch 3 dummy-iframe test catches before live plugins.
5. **Manual smoke gap** — bot-hq is desktop; UI tests are manual. Mitigate via Playwright + tauri-driver in Batch 5 for top 5–10 flows.

---

## Open items deferred

- Plugin auto-restart (exponential backoff) — defer until before third-party plugin authors get the SDK.
- Full UI-mutation plugin tier — defer until concrete use case.
- Linux + Windows builds — macOS first.
- Plugin author SDK + docs — separate design pass.
- Path B (force-flush on turn-end via `TurnEnded` variant) — revisit only if profiling shows perceived lag.
