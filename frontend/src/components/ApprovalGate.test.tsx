import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { ApprovalGate } from "./ApprovalGate";
import type { TrayRow } from "./HaltBanner";

const approval = (o: Partial<TrayRow> = {}): TrayRow => ({
  id: 1,
  choice_id: "c-1",
  agent: "hands",
  kind: "choice",
  prompt:
    "Run gated command in this session's repo?\n\n`gh pr create --base main`",
  options: ["Approve", "Reject"],
  status: "pending",
  asked_at: "2026-08-14T10:00:00Z",
  command_text: "gh pr create --base main",
  ...o,
});

const resolved = () => vi.fn().mockResolvedValue({ kind: "resolved" });

describe("ApprovalGate", () => {
  it("names who is blocked and shows the command verbatim", () => {
    render(
      <ApprovalGate
        rows={[approval()]}
        label={(a) => (a === "hands" ? "HANDS · Opus" : a)}
        onResolve={resolved()}
      />,
    );
    expect(screen.getByText("HANDS · Opus")).toBeInTheDocument();
    expect(screen.getByText("is blocked until you answer")).toBeInTheDocument();
    expect(screen.getByText("gh pr create --base main")).toBeInTheDocument();
  });

  it("resolves on the spot — one click, no Send", async () => {
    // The whole point of the gate. In the tray an approval had a Send button of
    // its own, which is what let the user answer a row and watch nothing move.
    const onResolve = resolved();
    render(<ApprovalGate rows={[approval()]} onResolve={onResolve} />);
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await waitFor(() =>
      expect(onResolve).toHaveBeenCalledWith("c-1", "Approve", false),
    );
  });

  it("rejects through the same path", async () => {
    const onResolve = resolved();
    render(<ApprovalGate rows={[approval()]} onResolve={onResolve} />);
    fireEvent.click(screen.getByRole("button", { name: "Reject" }));
    await waitFor(() =>
      expect(onResolve).toHaveBeenCalledWith("c-1", "Reject", false),
    );
  });

  it("keeps Pause reachable", async () => {
    // Pause is the only interrupt in the product (rc3 D33), and a gate is
    // exactly when a user might want it. A gate you cannot escape is how a
    // harness loses a user's trust.
    const onCancel = vi.fn().mockResolvedValue(undefined);
    render(
      <ApprovalGate
        rows={[approval()]}
        onResolve={resolved()}
        onCancel={onCancel}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    await waitFor(() => expect(onCancel).toHaveBeenCalledTimes(1));
  });

  it("shows one approval at a time and says how many follow", () => {
    // A user approving a push needs to read THAT push. Five commands stacked in
    // one pane is how a blind Approve happens.
    render(
      <ApprovalGate
        rows={[
          approval({ id: 1, choice_id: "c-1", command_text: "git push" }),
          approval({ id: 2, choice_id: "c-2", command_text: "rm -rf build" }),
        ]}
        onResolve={resolved()}
      />,
    );
    expect(screen.getByText("git push")).toBeInTheDocument();
    expect(screen.queryByText("rm -rf build")).toBeNull();
    expect(screen.getByText(/1 of 2 · 1 more after this/)).toBeInTheDocument();
  });

  it("asks again before running a command the backend flagged as stale", async () => {
    // The one irreversible mistake this surface can make: running a command
    // whose repo has moved under it. The backend refuses the blind approve and
    // this has to carry that refusal through rather than swallow it.
    const onResolve = vi
      .fn()
      .mockResolvedValueOnce({
        kind: "needs_stale_confirm",
        command: "git push --force",
        asked_at: "2026-08-14T02:00:00Z",
      })
      .mockResolvedValueOnce({ kind: "resolved" });
    render(<ApprovalGate rows={[approval()]} onResolve={onResolve} />);

    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await waitFor(() =>
      expect(screen.getByText(/confirm you still want this to run/i)).toBeInTheDocument(),
    );
    // The pick is NOT applied yet.
    expect(onResolve).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "Run it anyway" }));
    await waitFor(() =>
      expect(onResolve).toHaveBeenLastCalledWith("c-1", "Approve", true),
    );
  });

  it("lets the user back out of a stale confirm without answering", async () => {
    const onResolve = vi.fn().mockResolvedValue({
      kind: "needs_stale_confirm",
      command: "git push --force",
      asked_at: null,
    });
    render(<ApprovalGate rows={[approval()]} onResolve={onResolve} />);
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    // Back to the gate, still pending, nothing run.
    expect(screen.getByRole("button", { name: "Approve" })).toBeInTheDocument();
    expect(onResolve).toHaveBeenCalledTimes(1);
  });

  it("surfaces a failed resolve instead of leaving the gate silent", async () => {
    // Answering IS the action here. A swallowed error leaves the gate up with
    // no signal and the user pressing Approve again.
    const onResolve = vi.fn().mockRejectedValue(new Error("bridge down"));
    render(<ApprovalGate rows={[approval()]} onResolve={onResolve} />);
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("bridge down"),
    );
    // Still answerable — the failure was in the call, not in the decision.
    expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled();
  });

  it("prints a push gate's own question rather than command boilerplate", () => {
    // A push gate has no command_text and its prompt IS the question. The
    // action-gate branch would have shown its first line only, which for a push
    // gate is the whole thing anyway — this pins that they don't get crossed.
    render(
      <ApprovalGate
        rows={[
          approval({
            command_text: null,
            prompt: "Allow `git push` to `staging` in this session's repo?",
          }),
        ]}
        onResolve={resolved()}
      />,
    );
    expect(
      screen.getByText("Allow `git push` to `staging` in this session's repo?"),
    ).toBeInTheDocument();
  });

  it("does not repeat the command in the prompt line", () => {
    // An action-gate prompt is boilerplate + a fenced copy of the command, and
    // the command already has its own block. Printing the prompt whole showed
    // it twice.
    render(<ApprovalGate rows={[approval()]} onResolve={resolved()} />);
    expect(
      screen.getByText("Run gated command in this session's repo?"),
    ).toBeInTheDocument();
    // Exactly one rendering of the command itself.
    expect(screen.getAllByText("gh pr create --base main")).toHaveLength(1);
  });
});
