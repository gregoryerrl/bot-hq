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

  it("renders one override block per role, enumerated from list_roles", async () => {
    // The blocks are the ROLES the store is keyed by — not two fixed turn
    // slots. A third role gets a third block for free; the old panel could not
    // address one at all.
    mockBackend({}, [...ROLES, role({ id: 3, slug: "scribe", display_name: "SCRIBE" })]);
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: /core knobs/i }));

    for (const name of ["HANDS", "EYES", "SCRIBE"]) {
      expect(
        await screen.findByRole("combobox", { name: `${name} effort level` }),
      ).toBeInTheDocument();
    }
    // …and nothing is offered under an agent's name or a turn slot.
    const pane = screen.getByText("Agent runtime overrides").parentElement!;
    expect(pane.textContent).not.toMatch(/\bbrian\b/i);
    expect(pane.textContent).not.toMatch(/\brain\b/i);
    expect(pane.textContent).not.toMatch(/turn 1/i);
  });

  it("writes an effort override under the ROLE SLUG spawn resolves, not another role's", async () => {
    // The whole point of the panel: land the value where
    // `resolve_agent_overrides` will look for it. `eyes` is deliberately the
    // SECOND role, so a block that wrote a fixed key would land on `hands`.
    mockBackend();
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: /core knobs/i }));

    fireEvent.change(
      await screen.findByRole("combobox", { name: "EYES effort level" }),
      { target: { value: "max" } },
    );
    fireEvent.click(await screen.findByRole("button", { name: /save changes/i }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "set_claude_overrides",
        expect.objectContaining({
          overrides: expect.objectContaining({
            per_role: { eyes: expect.objectContaining({ effort: "max" }) },
          }),
        }),
      ),
    );
  });

  it("keeps two same-named roles apart, and writes each under its own slug", async () => {
    // `roles.display_name` carries no UNIQUE constraint (only `roles.slug`
    // does), so the title alone cannot say which entry a block writes.
    mockBackend({}, [
      role({ id: 1, slug: "hands", display_name: "HANDS" }),
      role({ id: 2, slug: "hands-review", display_name: "HANDS" }),
    ]);
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: /core knobs/i }));

    // The slug is what tells them apart on screen.
    expect(await screen.findByText("hands-review")).toBeInTheDocument();

    const efforts = await screen.findAllByRole("combobox", {
      name: "HANDS effort level",
    });
    expect(efforts).toHaveLength(2);
    fireEvent.change(efforts[1], { target: { value: "low" } });
    fireEvent.click(await screen.findByRole("button", { name: /save changes/i }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "set_claude_overrides",
        expect.objectContaining({
          overrides: expect.objectContaining({
            per_role: { "hands-review": expect.objectContaining({ effort: "low" }) },
          }),
        }),
      ),
    );
  });

  it("shows a stored per-role override in that role's block, and only there", async () => {
    mockBackend({ per_role: { eyes: { effort: "low" } } });
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: /core knobs/i }));

    expect(
      await screen.findByRole("combobox", { name: "EYES effort level" }),
    ).toHaveValue("low");
    expect(
      screen.getByRole("combobox", { name: "HANDS effort level" }),
    ).toHaveValue("");
  });

  it("says so when nothing is stored per role, rather than showing blank as your config", async () => {
    // An override written before the re-key was keyed by agent name; serde drops
    // the unknown field on read, so the panel would otherwise render the loss as
    // an ordinary all-inherited state.
    mockBackend({ _all: { effort: "high" } });
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: /core knobs/i }));

    expect(await screen.findByRole("status")).toHaveTextContent(
      /no per-role override is stored/i,
    );
  });

  it("drops the notice once a role is configured", async () => {
    mockBackend({ per_role: { hands: { effort: "low" } } });
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: /core knobs/i }));

    await screen.findByRole("combobox", { name: "HANDS effort level" });
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("points at the Roles tab when there are no roles to configure", async () => {
    mockBackend({}, []);
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: /core knobs/i }));

    expect(await screen.findByText(/no roles yet/i)).toBeInTheDocument();
  });
});
