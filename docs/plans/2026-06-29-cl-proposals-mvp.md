# CL Proposals MVP Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a project-scoped Context Library proposal queue so agents can propose CL edits without directly mutating CL files.

**Architecture:** Store proposals in a durable `cl_proposals` table keyed by project and public `proposal_uid`. Agents create/read proposals via non-mutating MCP tools (`cl_propose`, `cl_list_proposals`); host-mediated approval writes supported `add`/`correct` changes atomically and rescans the CL. The polished frontend queue is deferred.

**Tech Stack:** Rust, sqlx/sqlite migrations, bot-hq internal MCP JSON-RPC, existing `SignalingBridge` and `Storage` patterns.

---

### Task 1: Storage schema and storage API

**Files:**
- Create: `migrations/0025_cl_proposals.sql`
- Create: `src/storage/cl_proposals.rs`
- Modify: `src/storage/row_types.rs`
- Modify: `src/storage/mod.rs`

**Step 1: Write failing storage tests**

Add tests in `src/storage/cl_proposals.rs` for:
- create/list/get/resolve proposal
- project/status scoping
- nullable `session_id`

**Step 2: Run focused test to verify RED**

Run: `cargo test storage::cl_proposals --lib`
Expected: compile failure / missing module or method.

**Step 3: Implement migration + row type + storage methods**

Add `cl_proposals` with project-scoped lifecycle and `open/approved/rejected` status. Implement minimal storage methods to pass tests.

**Step 4: Run focused test to verify GREEN**

Run: `cargo test storage::cl_proposals --lib`
Expected: tests pass.

### Task 2: MCP proposal creation and listing

**Files:**
- Create: `src/signaling/bridge/cl_proposals.rs`
- Modify: `src/signaling/bridge/mod.rs`
- Modify: `src/signaling/protocol.rs`
- Modify: `src/signaling/jsonrpc.rs`

**Step 1: Write failing bridge/MCP tests**

Add tests proving `cl_propose` creates a row and `cl_list_proposals(project, status?)` returns scoped results.

**Step 2: Run focused test to verify RED**

Run: `cargo test cl_proposals --lib`
Expected: failure due missing bridge/MCP methods.

**Step 3: Implement bridge + JSON-RPC + descriptors**

Add non-mutating MCP tools available to both agents. Validate proposal kind/status and return JSON/text consistent with surrounding tools.

**Step 4: Run focused test to verify GREEN**

Run: `cargo test cl_proposals --lib`
Expected: tests pass.

### Task 3: Host approval write-back

**Files:**
- Modify: `src/signaling/bridge/cl_proposals.rs` or add storage helpers as needed
- Optional later: Tauri command surface; polished UI deferred

**Step 1: Write failing tests**

Test `add` creates a missing file, `correct` replaces a full existing file, reject marks rejected without mutation, and delete approval is unsupported.

**Step 2: Run focused tests to verify RED**

Run: `cargo test cl_proposals --lib`
Expected: missing approval method failures.

**Step 3: Implement minimal host approval path**

Use existing CL root resolution, atomic temp-write + rename, and `cl_rescan(project)` after supported approval.

**Step 4: Run focused tests to verify GREEN**

Run: `cargo test cl_proposals --lib`
Expected: tests pass.

### Task 4: Full verification

Run final gates before commit:
- `cargo test`
- `cd frontend && npm test`
- `cd frontend && npm run lint`
- `cargo build --release`
- `cd frontend && npm run build`

Do not run global `cargo fmt`.
