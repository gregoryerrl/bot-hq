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

  it("does not offer a second way to answer an approval", async () => {
    // **Changed subject at rc3 D33.** This used to check the warning shown when
    // discarding an approval from the tray. Approvals are no longer answerable
    // here at all — they take the input slot (`ApprovalGate`), and leaving an
    // Approve button (or a Discard) in the tray as well would rebuild the exact
    // defect the gate exists to fix: two paths into one row, which is what
    // produced "I answered it and a second one appeared".
    //
    // Discard was also the wrong verb for a gate. Something is synchronously
    // blocked on the answer, so the explicit no is **Reject**, which tells the
    // hook. Discarding just walked away from a held-open command.
    await openTray([
      trayRow({
        command_text: "gh pr create --base staging",
        prompt: "Run gated command in this session's repo?",
        options: ["Approve", "Reject"],
      }),
    ]);
    // Present as a COUNT, so the tray is not silently missing a pending row…
    expect(await screen.findByText(/1 APPROVAL/)).toBeInTheDocument();
    expect(
      screen.getByText(/answered below the chat, where the input box is/i),
    ).toBeInTheDocument();
    // …but with no way to answer or dismiss it from here.
    expect(screen.queryByRole("button", { name: "Approve" })).toBeNull();
    expect(
      screen.queryByRole("button", {
        name: /discard this card without answering/i,
      }),
    ).toBeNull();
  });

  it("still lists an ordinary question, which IS parkable", async () => {
    // The division D33 draws: a question waits for you and the session carries
    // on, so it keeps its card, its options and its Discard. Only approvals —
    // where something is blocked right now — lose the tray.
    await openTray([
      trayRow({
        command_text: null,
        prompt: "Which branch should this target?",
        options: ["main", "staging"],
      }),
    ]);
    expect(
      await screen.findByText("Which branch should this target?"),
    ).toBeInTheDocument();
    expect(screen.queryByText(/APPROVAL/)).toBeNull();
    expect(
      screen.getByRole("button", {
        name: /discard this card without answering/i,
      }),
    ).toBeInTheDocument();
  });
});
