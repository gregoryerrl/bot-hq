# Decisions

Outputs of Phase 0 research tasks. Each section is the authoritative pick for one decision deferred to implementation per PLAN.md.

## slint

**Pin:** `slint = "1.16.1"` (latest 1.x as of 2026-04-23; no 2.x announced).

### Version & MSRV

- Latest stable on crates.io: **1.16.1** (released 2026-04-23). 1.16.0 dropped 2026-04-16; 1.16.1 is a patch (ListView compiler-panic fix, ComboBox eliding, two-way struct binding fix, Skia partial-render fix, macOS muda key-accelerators, LSP tracing).
- **MSRV is 1.92**, not 1.88. Workspace `rust-version = "1.92"` in slint-ui/slint master. We're on 1.95, comfortably clear. Pin `1.16.1` (or float `slint = "1.16"`) — keeping the patch component lets us absorb future 1.16.x fixes without code churn.
- No 2.x branch is announced or in-progress (Unreleased section in CHANGELOG is empty). Safe to pin on 1.x for the foreseeable future.

### State pattern — Slint Globals (confirmed)

Use a single `AppState` global declared in `ui/app.slint`. This is the canonical Slint pattern for project-wide shared state; callback prop-drilling through nested components is the wrong choice for an app of our size.

```slint
// ui/app.slint
export global AppState {
    in-out property <int> current-tab;       // 0..3 for Dashboard/CL/Settings/Emma
    in-out property <bool> emma-open;
    in-out property <[SessionTile]> sessions;
    callback open-session(string);
    callback broadcast(string, string);      // session-id, text
}
```

```rust
// src/main.rs / src/ui/view_model.rs
let app = App::new()?;
let state = app.global::<AppState>();
state.set_current_tab(0);
state.on_open_session({
    let app_weak = app.as_weak();
    move |sid| { /* dispatch to core */ }
});
```

- Access from Rust: `app.global::<AppState>()` → use `set_<prop>()` / `on_<callback>()` / `get_<prop>()`.
- Globals are per-window singletons; we only have one window, so this is fine.
- All cross-pane communication (Dashboard ↔ Session, Emma open/close, settings updates) flows through `AppState` properties/callbacks.

### List/grid views — `Model<T>` + `ModelRc<T>` + `VecModel<T>`

Confirmed. Dashboard tiles, sidebar session list, CL file tree, Brian/Rain chat lines all use this pattern.

```slint
// ui/dashboard.slint
export struct SessionTile { id: string, title: string, phase: string, awaiting: bool }
export component Dashboard {
    in property <[SessionTile]> tiles;
    for tile in tiles: TileView { data: tile; }
}
```

```rust
use slint::{ModelRc, VecModel};
use std::rc::Rc;

let tiles = Rc::new(VecModel::<SessionTile>::default());
tiles.push(SessionTile { id: "...".into(), title: "...".into(), phase: "I".into(), awaiting: false });
app.global::<AppState>().set_sessions(ModelRc::from(tiles.clone()));
// Later mutations on `tiles` (push/insert/remove/set_row_data) auto-refresh the UI via ModelTracker.
```

- `VecModel<T>` mutators (`push`, `insert`, `remove`, `set_row_data`, `clear`) take `&self` — interior mutability. Hold `Rc<VecModel<T>>` in the view-model layer to push updates from async core events.
- Wrap with `ModelRc::from(Rc<VecModel<T>>)` once when handing to Slint.
- For derived/filtered views use `MapModel` / `FilterModel` — but our needs are simple enough that direct `VecModel` mutation from the core event loop is fine.

### Backend init — optional, skip it

`Backend::set()` / `BackendSelector` is **optional**. Slint auto-selects (Winit + femtovg on macOS/Linux/Windows by default). Only reach for `BackendSelector` if we need to force a renderer (Skia, Software) or override via code instead of `SLINT_BACKEND` env var. We don't.

Typical `main()`:

```rust
slint::include_modules!();   // pulls in compiled .slint via build.rs

fn main() -> Result<(), slint::PlatformError> {
    let app = App::new()?;
    // wire globals / callbacks / models here
    app.run()  // = show() + event-loop + hide()
}
```

- `build.rs`: `slint_build::compile("ui/app.slint")?;`
- `run()` blocks on the main thread (required on macOS — event loop must be main-thread).
- For async core work (tokio runtime, subprocess IO, sqlx), spawn a tokio runtime on a separate thread and bridge events back via `slint::invoke_from_event_loop(...)` or `Weak<App>::upgrade_in_event_loop(...)`. This is the load-bearing pattern for our agent-event → UI updates.

### Gotchas for our app shape

1. **Main-thread event loop.** All Slint UI mutations must happen on the main thread. Use `slint::invoke_from_event_loop` from tokio tasks to push agent events into Models. Don't try to mutate `VecModel` from a tokio worker directly — it'll compile (interior mutability) but races the renderer.
2. **Responsive stacking (<1200px Brian/Rain).** Slint supports `if` element guards: `if root.width < 1200px: VerticalLayout { ... }` alongside `if root.width >= 1200px: HorizontalLayout { ... }`. No media queries — just property-bound conditionals. Each branch is a separate subtree; keep them thin or factor a shared child component.
3. **Modal/overlay slide-in (Emma).** Use a `PopupWindow` (built-in modal) or a sibling element with `z: 100` + animated `x` property (`animate x { duration: 200ms; easing: ease-out; }`). PopupWindow is easier but less flexible; the animated-x overlay sibling matches our half-pane slide-over spec better.
4. **Tabs.** Don't use the stdlib `TabWidget` for our topbar — its visual style won't match. Build a custom `HorizontalLayout` of buttons bound to `AppState.current-tab`, with content panes rendered via `if AppState.current-tab == N: PaneN {}`.
5. **Sidebar resize.** Slint doesn't have a built-in splitter. Implement via `TouchArea` on a 4px-wide divider that mutates a `sidebar-width` property on drag. ~30 lines of .slint.
6. **`Rc` not `Arc` for models.** Slint is single-threaded by design; `VecModel` is `!Send`. Bridge to your `Arc<AppState>` core via the event-loop hop above, not by trying to share the model itself across threads.
7. **`include_modules!()` vs `slint!{}` macro.** Use `include_modules!()` + `build.rs` for our case (multiple .slint files in `ui/`). The inline `slint!{}` macro is hello-world only.

## mcp-server

**Pick:** `rmcp` v1.7.0 (official `modelcontextprotocol/rust-sdk`, released 2026-05-13).

**Rationale.** `rmcp` is the upstream vendor's officially-maintained Rust SDK (repo: `modelcontextprotocol/rust-sdk`), actively shipping with v1.7.0 cut the day before this decision. It meets every criterion: (1) maintained — fresh release yesterday, 4.7M+ downloads; (2) first-class server mode via `ServerHandler` + `#[tool]`/`#[tool_router]` macros; (3) ships both stdio (`rmcp::transport::stdio()`) and Streamable HTTP (`StreamableHttpService`) transports — claude-code's `--mcp-config` consumes both via `type: "stdio"` / `type: "http"` shapes; (4) Rust edition 2021 compatible (works on stable 1.95); (5) lean dep tree under feature gates — we pull `server + transport-io + macros` only, no reqwest/TLS bloat. Hand-rolling JSON-RPC would duplicate work the SDK already covers (initialize handshake, `tools/list`, `tools/call`, schema generation via `schemars`, error frames). Chosen transport: **stdio, one MCP child process per claude-code agent**. Rationale: `--mcp-config` files declaring `type: "stdio"` cause claude-code to spawn the binary as a subprocess; our app re-execs its own binary with a `--mcp-server` subcommand flag and bridges the stdio handlers back into `AppState` via a Unix-domain socket. (HTTP would let one server serve all agents, but stdio keeps lifetimes 1:1 with the owning agent and avoids opening a local port.)

**Cargo.toml dep line:**

```toml
rmcp = { version = "1.7", features = ["server", "transport-io", "macros"] }
schemars = "0.8"  # tool parameter JSON-Schema derivation (rmcp re-exports the macro)
```

**`mcp-config.json` shape consumed by `claude --mcp-config <path>`** (one config file per agent spawn, written to a temp file by `src/agents/spawn.rs`):

```json
{
  "mcpServers": {
    "bot-hq-signaling": {
      "type": "stdio",
      "command": "/path/to/bot-hq",
      "args": ["--mcp-server", "--session-id", "<uuid>", "--agent", "brian"],
      "env": {
        "BOT_HQ_IPC_SOCKET": "/tmp/bot-hq-<pid>.sock"
      }
    }
  }
}
```

Spawn invocation pairs this with `--strict-mcp-config` so the user's global `~/.claude.json` MCP servers do not bleed into agent subprocesses.

**Tool-registration sketch (Rust)** in `src/signaling/mod.rs`:

```rust
use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router,
           ServiceExt, transport::stdio};

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct AskUserChoiceParams {
    question: String,
    options: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct MarkAwaitingUserParams { reason: String }

#[derive(Clone)]
struct Signaling { bridge: Arc<SignalingBridge> }  // bridge -> AppState via channels

#[tool_router(server_handler)]
impl Signaling {
    #[tool(description = "Ask the user to pick one option; blocks until they choose.")]
    async fn ask_user_choice(&self, Parameters(p): Parameters<AskUserChoiceParams>) -> String {
        self.bridge.ask_user_choice(p.question, p.options).await  // awaits oneshot
    }

    #[tool(description = "Flag this session as awaiting user input (non-blocking).")]
    async fn mark_awaiting_user(&self, Parameters(p): Parameters<MarkAwaitingUserParams>) {
        self.bridge.mark_awaiting_user(p.reason);
    }
}

pub async fn run_mcp_server(bridge: Arc<SignalingBridge>) -> anyhow::Result<()> {
    let service = Signaling { bridge }.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
```

Under the hood `rmcp` handles the standard JSON-RPC methods (`initialize`, `notifications/initialized`, `tools/list`, `tools/call`, `ping`) — we never write them by hand.

**Gotchas:**
- **Never `println!` from the MCP subprocess.** Stdout is the JSON-RPC channel; all logging must use `tracing-subscriber` with `.with_writer(std::io::stderr)`. Install a panic hook that routes to stderr too.
- **Subprocess <-> AppState bridge.** Because the MCP server runs in a *separate process* (claude-code spawns it as a child), `SignalingBridge` cannot be a plain in-memory `Arc<AppState>`. Two options: (a) Unix-domain socket at `BOT_HQ_IPC_SOCKET` carrying a tiny request/response protocol the parent GUI listens on; (b) switch to in-process HTTP transport on `127.0.0.1:<rand>` so the GUI process *is* the MCP server and agents connect via `type: "http"`. Phase 3 should re-evaluate — HTTP avoids the IPC layer at the cost of a localhost port and per-agent URL bookkeeping. Default: start with stdio + UDS bridge; promote to HTTP if the IPC layer grows hairy.
- **`type: "stdio"` is the literal JSON value claude-code expects.** `"sse"` is deprecated; `"streamable-http"` is an alias for `"http"`. Stick to `"stdio"` / `"http"`.
- **`MCP_TIMEOUT` env var** governs startup timeout — set generously (e.g. `30000`) during dev since cold-start of our binary may be slow.
- **Tool output cap:** claude-code warns past 10k tokens and truncates at `MAX_MCP_OUTPUT_TOKENS`. Our two tools return tiny strings, so non-issue, but worth noting if we add tools later.
- **rmcp version churn.** SDK is 1.x and shipping fast (1.7.0 yesterday); pin `rmcp = "=1.7.0"` initially and bump deliberately. Macro syntax changed between 0.x and 1.x — do not copy older blog-post examples without verifying they match 1.7.

## auth-storage

**v1 pick:** Plaintext sqlite columns in the `agent_configs` table (e.g. `anthropic_api_key TEXT`, `deepseek_api_key TEXT`). No encryption, no OS keychain integration.

### v1 rationale

- Speed of development. We ship single-binary install with one SQL migration; no platform-specific code paths in v1.
- No new runtime dependencies. The OS keychain story differs per platform (macOS Security framework, Windows wincred, Linux dbus + Secret Service daemon), and each pulls C ABI / dbus into the build matrix.
- Mirrors how most upstream-API tooling stores keys today (env var or dotfile). Users already accept that risk on dev machines.

### v1 risks (documented, accepted)

- Anything that backs up `~/.bot-hq/` (Time Machine, cloud sync of home dir, rsync to NAS) captures plaintext tokens. The sqlite file is at `~/.bot-hq/projects/<project>/bot-hq.db` and gets user-only mode bits (`0600`) but is not encrypted.
- Multi-user macOS / shared workstation: if `~/.bot-hq` ends up world-readable through misconfiguration, tokens leak. Single-user developer workstations are the design target; multi-tenant hosts are explicitly out of scope for v1.
- A compromised local process running as the bot-hq user can `cat` the db. Same threat model as `.env` files or `~/.aws/credentials`.

### v2 upgrade path — `keyring-core`

The Rust keyring ecosystem split in v4 (May 2026). The legacy `keyring` crate (3.x) is deprecated for new applications; `keyring-core` is the API surface and platform stores live in separate crates. We target `keyring-core` directly.

API sketch (from `keyring_core::Entry`):

```rust
// service = "bot-hq", user = format!("{project}:{agent}:{provider}")
let entry = keyring_core::Entry::new("bot-hq", &user_key)?;
entry.set_password(&api_key)?;       // store
let key = entry.get_password()?;     // fetch
entry.delete_credential()?;          // revoke
```

Backends per platform: macOS Keychain Services, Windows Credential Manager, Linux Secret Service (dbus). The application picks a default store via `keyring_core::set_default_store(...)` at startup; individual store crates (one per backend) are added as conditional `cfg(target_os = ...)` deps. Error variants we handle: `NoEntry`, `NoDefaultStore`, `Ambiguous`, `BadEncoding`, `Invalid`.

### Migration sketch (v1 → v2)

On first launch of a v2-capable build:
1. Read each non-NULL `*_api_key` column from `agent_configs`.
2. For each row, construct `Entry::new("bot-hq", &format!("{project}:{agent_id}:{provider}"))` and call `set_password`.
3. On success, `UPDATE agent_configs SET <col> = NULL WHERE rowid = ?` to scrub the plaintext.
4. Bump a `schema_version` row so the migration runs exactly once.
5. On subsequent launches, the columns are NULL and reads go through `keyring-core` directly.

If keychain access fails (Linux without Secret Service daemon, headless CI), fall back to plaintext-sqlite mode with a startup warning. v2 must remain installable in headless environments.

### README warning (v1)

> bot-hq stores per-agent API keys in `~/.bot-hq/projects/<project>/bot-hq.db` as plaintext. The file is created with `0600` permissions but is not encrypted. Anything that backs up your home directory (Time Machine, cloud sync, rsync) will capture these tokens. Treat the db file with the same care as `.env` or `~/.aws/credentials`. OS keychain integration is planned for v2 — see `docs/decisions.md#auth-storage`.

