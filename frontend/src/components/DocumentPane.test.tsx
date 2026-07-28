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

function renderTray(rows: SessionTrayView[]) {
  mockInvoke.mockImplementation((cmd: string) => {
    if (cmd === "list_session_tray") return Promise.resolve(rows);
    if (cmd === "discard_choice") return Promise.resolve(true);
    if (cmd === "compute_apply_diff")
      return Promise.resolve({ lines: [], note: null });
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

async function openTray(rows: SessionTrayView[]) {
  renderTray(rows);
  // The Tray tab is phase-independent and starts closed.
  fireEvent.click(await screen.findByRole("button", { name: /tray/i }));
}

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
