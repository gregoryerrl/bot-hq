import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ClaudeConfigPanel } from "./ClaudeConfig";
import { invoke } from "@tauri-apps/api/core";
import type { ClaudeOverrides, RoleView } from "../lib/bindings";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

const inh = (inherited: string[], skipped: string[]) => ({
  inherited_by: inherited,
  skipped_by: skipped,
  note: "note",
  overridable: true,
});

// What `claude_config::inheritance` actually emits: one collective chip, never
// an agent name (rc3 D10 replaced the two literals with this constant).
const EVERY_AGENT = "every agent";

// `claude_config::reader` labels MCP forwarding by CAPABILITY, not by name —
// `user_mcp_servers_for_agent` forwards to a role granting `edit_files` and to
// nobody else. Different string from the inheritance chips, on purpose.
const MCP_FORWARDED_TO = "agents that may edit files";

function role(over: Partial<RoleView> = {}): RoleView {
  return {
    id: 1,
    slug: "hands",
    display_name: "HANDS",
    description_prompt: null,
    capabilities: ["read_channel", "edit_files"],
    participation_mode: "active",
    default_model_id: null,
    builtin: false,
    archived: false,
    ...over,
  };
}

const ROLES: RoleView[] = [
  role(),
  role({ id: 2, slug: "eyes", display_name: "EYES" }),
];

const CONFIG = {
  config_dir: "/home/u/.claude",
  config_dir_source: "default (~/.claude)",
  home_claude_json: { present: true, path: "/home/u/.claude.json", bytes: 100 },
  managed_settings_present: false,
  core_knobs: [
    {
      key: "env.CLAUDE_CODE_EFFORT_LEVEL",
      label: "Effort level",
      value: "xhigh",
      source: "~/.claude/settings.json (effortLevel, legacy)",
      inheritance: inh([EVERY_AGENT], []),
    },
  ],
  skills: [
    {
      name: "my-skill",
      kind: "user",
      disable_model_invocation: true,
      description: "take notes",
      path: "/p/note/SKILL.md",
      inheritance: inh([EVERY_AGENT], []),
    },
  ],
  plugins: [
    { key: "alpha@mkt", enabled: true, inheritance: inh([EVERY_AGENT], []) },
  ],
  mcp_servers: [
    {
      name: "discord",
      transport: "stdio",
      loaded_from: "~/.claude.json",
      effective: true,
      detail: "npx tsx",
      forwarded_to_agents: [MCP_FORWARDED_TO],
      reserved_filtered: false,
    },
  ],
  memory: {
    user_claude_md: { present: true, path: "/c/CLAUDE.md", bytes: 10 },
    home_claude_md: { present: false, path: "/h/CLAUDE.md", bytes: 0 },
    projects_with_memory: 2,
    inheritance: inh([EVERY_AGENT], []),
  },
  permissions: {
    default_mode: "default",
    allow: 0,
    ask: 0,
    deny: 1,
    additional_directories: 0,
    inheritance: inh([], [EVERY_AGENT]),
  },
  warnings: ["a server lives only in settings.json"],
};

/** Wires every read the panel makes. */
function mockBackend(
  overrides: ClaudeOverrides = {},
  roles: RoleView[] = ROLES,
) {
  mockInvoke.mockImplementation(async (cmd: string) => {
    if (cmd === "claude_config_read") return CONFIG;
    if (cmd === "get_claude_overrides") return overrides;
    if (cmd === "list_roles") return roles;
    if (cmd === "list_sessions") return [];
    return undefined;
  });
}

function renderPanel() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <ClaudeConfigPanel />
    </QueryClientProvider>,
  );
}

describe("Claude Config panel", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("shows the resolved config dir and warnings on the overview", async () => {
    mockBackend();
    renderPanel();
    // config dir appears in both the sidebar header and the overview stat.
    expect((await screen.findAllByText("/home/u/.claude")).length).toBeGreaterThan(0);
    expect(
      screen.getByText(/a server lives only in settings\.json/i),
    ).toBeInTheDocument();
  });

  it("renders the inheritance lens on the skills surface", async () => {
    mockBackend();
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: /skills/i }));
    expect(await screen.findByText("my-skill")).toBeInTheDocument();
    expect(screen.getByText(`${EVERY_AGENT} inherits`)).toBeInTheDocument();
  });

  it("saves a per-agent skill override to the _all fan-out", async () => {
    mockBackend();
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: /skills/i }));

    const select = await screen.findByRole("combobox");
    fireEvent.change(select, { target: { value: "user-invocable-only" } });

    const save = await screen.findByRole("button", { name: /save changes/i });
    fireEvent.click(save);

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "set_claude_overrides",
        expect.objectContaining({
          overrides: expect.objectContaining({
            _all: expect.objectContaining({
              skills: { "my-skill": "user-invocable-only" },
            }),
          }),
        }),
      ),
    );
  });

  it("stages a global core-knob edit and flushes it on Save", async () => {
    mockBackend();
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: /core knobs/i }));

    // The effort knob is an enum select (global edit, writes settings.json).
    const select = await screen.findByDisplayValue("xhigh");
    fireEvent.change(select, { target: { value: "high" } });

    // Batched: nothing written until Save.
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "claude_config_set_string",
      expect.anything(),
    );
    fireEvent.click(await screen.findByRole("button", { name: /save changes/i }));

    // Effort routes through the env var, and the legacy field is cleared so it
    // can't shadow it.
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("claude_config_set_string", {
        key: "env.CLAUDE_CODE_EFFORT_LEVEL",
        value: "high",
      }),
    );
    expect(mockInvoke).toHaveBeenCalledWith("claude_config_set_string", {
      key: "effortLevel",
      value: null,
    });
  });

  it("offers no per-role effort editor — the Roles tab owns a role's default now", async () => {
    // No-inherit (2026-08-25): the per-role effort/ultracode blocks left this
    // tab. What remains on Core knobs is the user's OWN settings.json effort
    // row, whose note points at the Roles tab instead of claiming agents
    // inherit it.
    mockBackend({ per_role: { eyes: { effort: "low" } } });
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: /core knobs/i }));

    expect(await screen.findByText("Effort level")).toBeInTheDocument();
    expect(screen.queryByText(/agent runtime overrides/i)).toBeNull();
    expect(
      screen.queryByRole("combobox", { name: /effort level$/i }),
    ).toBeNull();
    // Both the pane blurb and the knob note point at the Roles tab.
    expect(screen.getAllByText(/settings → roles/i).length).toBeGreaterThan(0);
  });

  it("carries per_role entries through a save untouched", async () => {
    // The section that edited per_role is gone, but the store it edited is
    // shared with the Roles tab — a save from THIS tab (which only patches
    // `_all`) must not drop the role defaults stored beside it.
    mockBackend({
      per_role: { hands: { effort: "xhigh", ultracode: true } },
    });
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: /skills/i }));

    const select = await screen.findByRole("combobox");
    fireEvent.change(select, { target: { value: "user-invocable-only" } });
    fireEvent.click(await screen.findByRole("button", { name: /save changes/i }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "set_claude_overrides",
        expect.objectContaining({
          overrides: expect.objectContaining({
            _all: expect.objectContaining({
              skills: { "my-skill": "user-invocable-only" },
            }),
            per_role: {
              hands: expect.objectContaining({
                effort: "xhigh",
                ultracode: true,
              }),
            },
          }),
        }),
      ),
    );
  });
});
