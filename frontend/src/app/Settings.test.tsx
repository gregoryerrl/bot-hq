import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { Settings } from "./Settings";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

/**
 * `get_app_setting` is keyed, so the stub answers per key — otherwise the two
 * Session-defaults checkboxes would read each other's value and a test could
 * pass while the component asked for the wrong setting.
 */
function mockBackend(settings: Record<string, string | null> = {}) {
  mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === "list_roles") return [];
    if (cmd === "list_models") return [];
    if (cmd === "list_capabilities") return [];
    if (cmd === "get_general_policy") return {};
    if (cmd === "get_app_setting") {
      const key = (args as { key?: string } | undefined)?.key ?? "";
      return settings[key] ?? null;
    }
    if (cmd === "set_app_setting") return null;
    return undefined;
  });
}

function renderSettings() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <Settings />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

const subtab = (name: RegExp) => screen.queryByRole("button", { name });

/** Opens Policy, where the session-create defaults live since rc3 D8. */
async function openPolicy() {
  fireEvent.click(screen.getByRole("button", { name: /^policy$/i }));
  return screen.findByRole("heading", { name: /session defaults/i });
}

describe("Settings", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("has no Agents subtab", async () => {
    mockBackend();
    renderSettings();

    // rc3 D8: the tab is gone, not merely empty. Waiting on Roles first so the
    // absence is asserted against a rendered tab row, not an empty one.
    expect(await screen.findByRole("heading", { name: /^roles$/i, level: 1 }))
      .toBeInTheDocument();
    expect(subtab(/^agents$/i)).not.toBeInTheDocument();
  });

  it("lands on Roles", async () => {
    mockBackend();
    renderSettings();

    // The inactive panels are hidden by a Tailwind class, and jsdom loads no
    // stylesheet — so "which panel is visible" is not observable here. The
    // active pill's own styling is, and it is what the user sees.
    await screen.findByRole("heading", { name: /^roles$/i, level: 1 });
    expect(subtab(/^roles$/i)).toHaveClass("text-primary");
    expect(subtab(/^models$/i)).not.toHaveClass("text-primary");
  });

  // The two seeds are deliberately opposite ("0" = worktrees off, "1" = solo
  // on), so a box wired to the OTHER key reads the wrong state and fails.
  const SEEDS = { worktree_default: "0", rain_disabled_default: "1" };

  it("keeps the worktree session default reachable, under Policy", async () => {
    mockBackend(SEEDS);
    renderSettings();
    await openPolicy();

    const box = screen.getByRole("checkbox", {
      name: /isolated git worktrees/i,
    });
    // The heading renders before `get_app_setting` resolves, so the seeded
    // state has to be waited for — asserting straight away would only see the
    // pre-load default and pass whatever the box is wired to.
    await waitFor(() => expect(box).not.toBeChecked());

    fireEvent.click(box);
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_app_setting", {
        key: "worktree_default",
        value: "1",
      }),
    );
  });

  it("keeps the solo-by-default session default reachable, under Policy", async () => {
    mockBackend(SEEDS);
    renderSettings();
    await openPolicy();

    const box = screen.getByRole("checkbox", { name: /start solo/i });
    await waitFor(() => expect(box).toBeChecked());

    // Cleared, not "0": `default_rain_enabled` only treats the literal "1" as
    // solo, so an empty string is what turns it back off.
    fireEvent.click(box);
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_app_setting", {
        key: "rain_disabled_default",
        value: "",
      }),
    );
  });

  it("shows worktrees on when the setting was never written", async () => {
    // Opt-OUT: only the literal "0" turns worktrees off, so an unset row has
    // to read as checked.
    mockBackend({});
    renderSettings();
    await openPolicy();

    expect(
      screen.getByRole("checkbox", { name: /isolated git worktrees/i }),
    ).toBeChecked();
  });
});
