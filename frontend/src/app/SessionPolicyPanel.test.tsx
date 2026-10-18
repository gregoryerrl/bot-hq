import { render, screen, fireEvent, within } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { SessionPolicyPanel } from "./SessionPolicyPanel";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

/**
 * The session's roster, as `list_session_participants` returns it. Two rows
 * sharing a role with different models — the configuration the user could not
 * tell apart until the agents said so — plus a disabled row, because the
 * panel is the one surface that shows a participant that is not running.
 */
const ROSTER = [
  {
    id: 1,
    slug: "eyes",
    role_display_name: "EYES",
    model_display_name: "Claude Opus 5",
    turn_position: 0,
    participation_mode: "active",
    enabled: true,
  },
  {
    id: 2,
    slug: "eyes-2",
    role_display_name: "EYES",
    model_display_name: "DeepSeek R2",
    turn_position: 1,
    participation_mode: "on_mention",
    enabled: false,
  },
];

function mockBackend(participants: unknown[] = ROSTER) {
  mockInvoke.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "list_session_participants":
        return participants;
      case "get_session_policy":
        return {};
      case "get_session_tool_gate":
        return [];
      // Nullable read — React Query rejects an `undefined` result.
      default:
        return null;
    }
  });
}

function renderPanel() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <SessionPolicyPanel sessionId="s1" open onClose={() => {}} />
    </QueryClientProvider>,
  );
}

describe("SessionPolicyPanel roster (rc3 D10)", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("names every participant as ROLE · Model, resolved through the roster", async () => {
    // Tested as ONE chain: `list_session_participants` goes in, the rendered
    // roster block comes out. The panel had no test file at all, so swapping
    // `participantLabel(p)` for `p.slug` here survived the whole suite.
    mockBackend();
    renderPanel();

    expect(await screen.findByText("EYES · Claude Opus 5")).toBeInTheDocument();
    expect(screen.getByText("EYES-2 · DeepSeek R2")).toBeInTheDocument();
  });

  it("keeps the slugs — and any agent name — off the panel", async () => {
    mockBackend();
    renderPanel();
    await screen.findByText("EYES · Claude Opus 5");

    const panel = screen.getByRole("dialog", { name: /session settings/i });
    // `eyes-2` is the slug; `EYES-2 · DeepSeek R2` is what the same row prints,
    // so this fails the moment the block renders the internal key instead.
    expect(panel.textContent).not.toMatch(/eyes-2/);
    expect(panel.textContent).not.toMatch(/\bbrian\b/i);
    expect(panel.textContent).not.toMatch(/\brain\b/i);
  });

  it("still states each participant's turn order and standing", async () => {
    // The label is not the whole row: the panel is where a user checks WHO is
    // observing and who is switched off, so the label must not have displaced
    // the rest of the line.
    mockBackend();
    renderPanel();
    const label = await screen.findByText("EYES-2 · DeepSeek R2");

    const row = label.closest("li")!;
    expect(row.textContent).toMatch(/on_mention|on mention/i);
    expect(row.textContent).toMatch(/disabled/);
    // Turn order is the list order, and the enabled row comes first.
    const rows = Array.from(row.closest("ol")!.querySelectorAll("li"));
    expect(rows[0].textContent).toMatch(/EYES · Claude Opus 5/);
    expect(rows[1]).toBe(row);
  });

  it("shows no roster block at all when the session has no participants", async () => {
    // Nothing to state — an empty "Participants" header would imply the read
    // failed rather than that the roster is empty.
    mockBackend([]);
    renderPanel();
    // The drawer itself is what tells us the panel finished rendering.
    await screen.findByRole("dialog", { name: /session settings/i });

    expect(screen.queryByText(/participants \(turn order\)/i)).toBeNull();
  });
});

describe("SessionPolicyPanel — every way out asks before dropping unsaved edits (B3f)", () => {
  beforeEach(() => mockInvoke.mockReset());

  function renderWith(onClose: () => void, keywords: unknown[] = []) {
    mockInvoke.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "list_session_participants":
          return [];
        case "get_session_policy":
          return {};
        case "get_session_tool_gate":
          return keywords;
        default:
          return null;
      }
    });
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    return render(
      <QueryClientProvider client={qc}>
        <SessionPolicyPanel sessionId="s1" open onClose={onClose} />
      </QueryClientProvider>,
    );
  }

  const ask = () => screen.findByRole("dialog", { name: "Discard changes?" });

  it("closes at once when nothing is unsaved", async () => {
    const onClose = vi.fn();
    renderWith(onClose);
    fireEvent.click(await screen.findByRole("button", { name: "Close session settings" }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog", { name: "Discard changes?" })).toBeNull();
  });

  it("asks on the close button, on Escape and on the scrim, and closes only on Discard", async () => {
    const onClose = vi.fn();
    renderWith(onClose);
    fireEvent.change(await screen.findByPlaceholderText("(no constraint)"), {
      target: { value: "feature/.*" },
    });

    fireEvent.click(screen.getByRole("button", { name: "Close session settings" }));
    fireEvent.click(within(await ask()).getByRole("button", { name: "Cancel" }));

    fireEvent.keyDown(window, { key: "Escape" });
    await ask();
    // Escape answers the question with Cancel, and does not close the drawer.
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "Discard changes?" })).toBeNull();

    fireEvent.click(screen.getByTestId("session-settings-scrim"));
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.click(within(await ask()).getByRole("button", { name: "Discard changes" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("counts the session's keyword edits as unsaved too", async () => {
    const onClose = vi.fn();
    renderWith(onClose, [{ keyword: "gh pr merge", mode: "gate" }]);
    fireEvent.click(await screen.findByRole("button", { name: "Remove keyword" }));
    fireEvent.click(screen.getByRole("button", { name: "Close session settings" }));
    await ask();
    expect(onClose).not.toHaveBeenCalled();
  });
});
