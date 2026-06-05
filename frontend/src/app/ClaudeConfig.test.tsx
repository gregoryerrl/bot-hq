import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ClaudeConfigPanel } from "./ClaudeConfig";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

const inh = (inherited: string[], skipped: string[]) => ({
  inherited_by: inherited,
  skipped_by: skipped,
  note: "note",
  overridable: true,
});

const CONFIG = {
  config_dir: "/home/u/.claude",
  config_dir_source: "default (~/.claude)",
  home_claude_json: { present: true, path: "/home/u/.claude.json", bytes: 100 },
  managed_settings_present: false,
  core_knobs: [
    {
      key: "effortLevel",
      label: "Effort level",
      value: "xhigh",
      source: "~/.claude/settings.json",
      inheritance: inh(["brian", "rain"], []),
    },
  ],
  skills: [
    {
      name: "note",
      kind: "user",
      disable_model_invocation: true,
      description: "take notes",
      path: "/p/note/SKILL.md",
      inheritance: inh(["brian"], ["rain"]),
    },
  ],
  plugins: [{ key: "warp@mkt", enabled: true, inheritance: inh(["brian"], ["rain"]) }],
  mcp_servers: [
    {
      name: "discord",
      transport: "stdio",
      loaded_from: "~/.claude.json",
      effective: true,
      detail: "npx tsx",
      forwarded_to_agents: ["brian"],
      reserved_filtered: false,
    },
  ],
  memory: {
    user_claude_md: { present: true, path: "/c/CLAUDE.md", bytes: 10 },
    home_claude_md: { present: false, path: "/h/CLAUDE.md", bytes: 0 },
    projects_with_memory: 2,
    inheritance: inh(["brian"], ["rain"]),
  },
  permissions: {
    default_mode: "default",
    allow: 0,
    ask: 0,
    deny: 1,
    additional_directories: 0,
    inheritance: inh([], ["brian", "rain"]),
  },
  warnings: ["a server lives only in settings.json"],
};

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
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "claude_config_read") return CONFIG;
      if (cmd === "get_claude_overrides") return {};
      if (cmd === "list_sessions") return [];
      return undefined;
    });
    renderPanel();
    // config dir appears in both the sidebar header and the overview stat.
    expect((await screen.findAllByText("/home/u/.claude")).length).toBeGreaterThan(0);
    expect(
      screen.getByText(/a server lives only in settings\.json/i),
    ).toBeInTheDocument();
  });

  it("renders the inheritance lens on the skills surface", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "claude_config_read") return CONFIG;
      if (cmd === "get_claude_overrides") return {};
      if (cmd === "list_sessions") return [];
      return undefined;
    });
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: /skills/i }));
    expect(await screen.findByText("note")).toBeInTheDocument();
    expect(screen.getByText("brian inherits")).toBeInTheDocument();
    expect(screen.getByText("rain skips")).toBeInTheDocument();
  });

  it("saves a per-agent skill override to the _all fan-out", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "claude_config_read") return CONFIG;
      if (cmd === "get_claude_overrides") return {};
      if (cmd === "list_sessions") return [];
      return undefined;
    });
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
              skills: { note: "user-invocable-only" },
            }),
          }),
        }),
      ),
    );
  });

  it("stages a global core-knob edit and flushes it on Save", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "claude_config_read") return CONFIG;
      if (cmd === "get_claude_overrides") return {};
      if (cmd === "list_sessions") return [];
      return undefined;
    });
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

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("claude_config_set_string", {
        key: "effortLevel",
        value: "high",
      }),
    );
  });
});
