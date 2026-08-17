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

  it("never scrolls the command sideways — a half-read command is a half-read approval", () => {
    // The house rule is vertical-only scrolling, and this is the surface where
    // breaking it costs the most: the user approves what they can SEE, and a
    // long command ran off the right edge of a `overflow-auto` box. The pair is
    // load-bearing — a bare `overflow-y-auto` still computes `overflow-x` to
    // `auto` — so both halves are asserted, plus the wrap that gives the text
    // somewhere to go.
    const long =
      "gh pr create --base main --head feat/a-very-long-branch-name --title 'x' --body 'y'";
    render(
      <ApprovalGate rows={[approval({ command_text: long })]} onResolve={resolved()} />,
    );
    const box = screen.getByText(long);
    expect(box.className).toContain("overflow-y-auto");
    expect(box.className).toContain("overflow-x-hidden");
    expect(box.className).not.toMatch(/(^|\s)overflow-auto(\s|$)/);
    expect(box.className).toContain("whitespace-pre-wrap");
    expect(box.className).toContain("break-all");
  });

  // issues.md #1 (2026-08-17): the tray card could open the file a gated
  // command names; the D33 gate could not, and the user approved seven
  // `--body-file /tmp/*.md` bodies unseen that day. The button is the wire from
  // the gate to the same viewer, on the card AND in Details.
  it("offers to view the file a gated command names, on the card and in Details", async () => {
    const onViewFile = vi.fn();
    render(
      <ApprovalGate
        rows={[
          approval({
            command_text:
              "gh pr merge 524 --squash --body-file /tmp/merge-524.md",
          }),
        ]}
        onResolve={resolved()}
        onViewFile={onViewFile}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "View merge-524.md" }));
    expect(onViewFile).toHaveBeenCalledWith("/tmp/merge-524.md");
    fireEvent.click(screen.getByRole("button", { name: "Details" }));
    // Two buttons now: the card's and the dialog's, both for the same path.
    const buttons = screen.getAllByRole("button", { name: "View merge-524.md" });
    expect(buttons).toHaveLength(2);
    fireEvent.click(buttons[1]);
    expect(onViewFile).toHaveBeenLastCalledWith("/tmp/merge-524.md");
    expect(onViewFile).toHaveBeenCalledTimes(2);
  });

  it("offers no file button when the command names none, or no viewer is wired", () => {
    const { unmount } = render(
      <ApprovalGate rows={[approval()]} onResolve={resolved()} onViewFile={vi.fn()} />,
    );
    expect(screen.queryByRole("button", { name: /^View / })).toBeNull();
    unmount();
    render(
      <ApprovalGate
        rows={[approval({ command_text: "gh pr create --body-file pr-body1.md" })]}
        onResolve={resolved()}
      />,
    );
    expect(screen.queryByRole("button", { name: /^View / })).toBeNull();
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

  it("Details surfaces absolutely everything about the gate", () => {
    // vision.md: "Full transparency. Every bit of information the agents see
    // is visible to the user." The card clamps the command to a short scroll
    // box and shows only the prompt's first line — a PR-create gate carries
    // its whole PR body inside that command. Details opens the lot: every
    // recorded field plus the untruncated request and command.
    const body = "## Summary — closes the guard gap the suite proved.";
    render(
      <ApprovalGate
        rows={[
          approval({
            prompt:
              "Run gated command in this session's repo?\n\nContext the card never shows.",
            command_text: `gh pr create --base main --title 'fix: close the gap' --body '${body}'`,
          }),
        ]}
        onResolve={resolved()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Details" }));
    const dialog = screen.getByRole("dialog", { name: "Approval details" });
    expect(dialog).toHaveTextContent("hands");
    expect(dialog).toHaveTextContent("2026-08-14T10:00:00Z");
    expect(dialog).toHaveTextContent("c-1");
    expect(dialog).toHaveTextContent("Approve / Reject");
    expect(dialog).toHaveTextContent("Context the card never shows.");
    expect(dialog).toHaveTextContent("closes the guard gap the suite proved");
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
