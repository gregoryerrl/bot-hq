import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { DocumentPane } from "./DocumentPane";
import type { SessionTrayView } from "../lib/bindings";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const mockInvoke = vi.mocked(invoke);

function trayRow(over: Partial<SessionTrayView> = {}): SessionTrayView {
  return {
    id: 1,
    session_id: "s1",
    choice_id: "c-1",
    agent: "brian",
    kind: "choice",
    prompt: "Ship it?",
    options: ["Yes", "No"],
    status: "pending",
    picked_option: null,
    asked_at: "2026-07-29T00:00:00Z",
    answered_at: null,
    supersedes_id: null,
    command_text: null,
    stale: false,
    ...over,
  } as SessionTrayView;
}

/** The session's roster, as `list_session_participants` returns it (rc3 D10). */
const ROSTER = [
  {
    id: 1,
    slug: "hands",
    role_display_name: "HANDS",
    model_display_name: "Claude Opus 5",
    turn_position: 0,
    participation_mode: "active",
    enabled: true,
  },
];

function renderTray(rows: SessionTrayView[], participants: unknown[] = []) {
  mockInvoke.mockImplementation((cmd: string) => {
    if (cmd === "list_session_tray") return Promise.resolve(rows);
    if (cmd === "discard_choice") return Promise.resolve(true);
    if (cmd === "compute_apply_diff")
      return Promise.resolve({ lines: [], note: null });
    if (cmd === "list_session_participants") return Promise.resolve(participants);
    // session_doc_search and friends are list-shaped; the pane filters them.
    return Promise.resolve([]);
  });
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <DocumentPane sessionId="s1" sessionPhase="apply" />
    </QueryClientProvider>,
  );
}

async function openTray(rows: SessionTrayView[], participants: unknown[] = []) {
  renderTray(rows, participants);
  // The Tray tab is phase-independent and starts closed.
  fireEvent.click(await screen.findByRole("button", { name: /tray/i }));
}

describe("tray card attribution (rc3 D10)", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("names who asked as ROLE · Model, resolved through the session's roster", async () => {
    // Tested as ONE chain: the stored `agent` slug goes through
    // `list_session_participants` and comes out as the card's asked-by line.
    // The tray was named in the D10 sweep list and was still printing
    // `entry.agent` straight from the row.
    await openTray([trayRow({ agent: "hands" })], ROSTER);

    expect(await screen.findByText("Ship it?")).toBeInTheDocument();
    expect(screen.getByText("HANDS · Claude Opus 5")).toBeInTheDocument();
    // The slug is an internal key; it must not reach the card.
    expect(screen.queryByText("hands")).toBeNull();
  });

  it("does not print a legacy agent name when the roster cannot place it", async () => {
    // A card parked before the rekey keeps `agent = 'brian'` forever — rc3 D10
    // kept that history readable on purpose, but not by that name.
    await openTray([trayRow({ agent: "brian" })], ROSTER);
    await screen.findByText("Ship it?");

    expect(screen.getByText("Unknown participant")).toBeInTheDocument();
    expect(screen.queryByText(/^brian$/i)).toBeNull();
  });
});

describe("tray discard", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("confirms before discarding, then calls discard_choice", async () => {
    await openTray([trayRow()]);
    fireEvent.click(
      await screen.findByRole("button", {
        name: /discard this card without answering/i,
      }),
    );

    // Confirm step exists — a mis-click must not destroy a live question.
    expect(await screen.findByText(/discard this card\?/i)).toBeInTheDocument();
    expect(mockInvoke).not.toHaveBeenCalledWith("discard_choice", expect.anything());

    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("discard_choice", {
        choiceId: "c-1",
      }),
    );
  });

  it("never resolves the choice — discarding is not an answer", async () => {
    await openTray([trayRow()]);
    fireEvent.click(
      await screen.findByRole("button", {
        name: /discard this card without answering/i,
      }),
    );
    fireEvent.click(await screen.findByRole("button", { name: "Discard" }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("discard_choice", {
        choiceId: "c-1",
      }),
    );
    // The whole point of the trash button: nothing is sent back to the agent.
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "resolve_choice",
      expect.anything(),
    );
  });

  it("cancelling the confirm leaves the card alone", async () => {
    await openTray([trayRow()]);
    fireEvent.click(
      await screen.findByRole("button", {
        name: /discard this card without answering/i,
      }),
    );
    fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));

    expect(mockInvoke).not.toHaveBeenCalledWith(
      "discard_choice",
      expect.anything(),
    );
    expect(screen.getByText("Ship it?")).toBeInTheDocument();
  });

  it("warns that discarding an approval gate means not-approved", async () => {
    await openTray([
      trayRow({
        command_text: "gh pr create --base staging",
        prompt: "Run gated command in this session's repo?",
        options: ["Approve", "Reject"],
      }),
    ]);
    fireEvent.click(
      await screen.findByRole("button", {
        name: /discard this card without answering/i,
      }),
    );
    expect(await screen.findByText(/not approved/i)).toBeInTheDocument();
  });
});
