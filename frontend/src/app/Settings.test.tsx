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

const subtab = (name: RegExp) => screen.queryByRole("tab", { name });

/** Opens Policy, where the session-create defaults live since rc3 D8. */
async function openPolicy() {
  fireEvent.click(screen.getByRole("tab", { name: /^policy$/i }));
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

  // Worktrees seeded OFF, against the opt-out default, so a box that ignores
  // the seed reads the wrong state and fails. The solo-by-default seed is
  // still supplied: rc3 D13 deleted the toggle, and the test below proves
  // Settings no longer reaches for the key even when it would answer.
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

  it("no longer offers a solo-by-default toggle, or reads its key (D13)", async () => {
    // The user: "There's no 'disable rain by default' on rc3, thats moot. Just
    // don't add the role to your session creation." It was a toggle only while
    // the roster was fixed at two; now the New-session dialog picks the roster,
    // so starting solo is just not adding a second participant.
    mockBackend(SEEDS);
    renderSettings();
    await openPolicy();
    // The sibling toggle is the render probe — waiting on it means the block
    // finished, so a missing checkbox below is absence, not a slow load.
    await waitFor(() =>
      expect(
        screen.getByRole("checkbox", { name: /isolated git worktrees/i }),
      ).not.toBeChecked(),
    );

    expect(
      screen.queryByRole("checkbox", { name: /one participant/i }),
    ).toBeNull();
    // Stronger than the missing box: nothing in Settings reaches for the key
    // at all, so a re-added hidden reader fails here too.
    expect(mockInvoke).not.toHaveBeenCalledWith("get_app_setting", {
      key: "rain_disabled_default",
    });
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

  it("offers the adherence-nudges opt-out, keyed on its own setting (round 8)", async () => {
    // `adherence_nudges` was read in four places and settable nowhere but
    // SQLite. Opt-OUT like worktrees: unset reads as on, "0" as off, and the
    // write goes to ITS key, not the worktree one.
    mockBackend({ adherence_nudges: "0" });
    renderSettings();
    await openPolicy();
    const box = screen.getByRole("checkbox", { name: /adherence nudges/i });
    await waitFor(() => expect(box).not.toBeChecked());
    fireEvent.click(box);
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_app_setting", {
        key: "adherence_nudges",
        value: "1",
      }),
    );
  });
});
