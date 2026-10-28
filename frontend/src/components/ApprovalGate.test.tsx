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

  // Feedback #52: an URGENT gate says so in red, reason first; an ordinary
  // gate shows nothing of the kind.
  it("shows an urgent gate's reason in red, and nothing on an ordinary one", () => {
    const { rerender } = render(
      <ApprovalGate rows={[approval({ urgent_reason: "the nightly job is burning the shared budget" })]} onResolve={resolved()} />,
    );
    expect(screen.getByText("URGENT APPROVAL")).toBeInTheDocument();
    expect(screen.getByTestId("gate-urgent-reason")).toHaveTextContent(
      "the nightly job is burning the shared budget",
    );
    expect(screen.getByRole("group", { name: "Approval required" }).className).toMatch(/border-error/);
    rerender(<ApprovalGate rows={[approval()]} onResolve={resolved()} />);
    expect(screen.getByText("APPROVAL")).toBeInTheDocument();
    expect(screen.queryByTestId("gate-urgent-reason")).not.toBeInTheDocument();
  });

  // Feedback #143: the reviewer's read is a structured line on the card —
  // who and when — not only a sentence inside the prompt; a skipped read and
  // a push gate (no reviewer step) say so; a row with no recorded provenance
  // shows nothing rather than calling the card unreviewed.
  it("shows the review provenance as data: reviewed by, skipped, or no reviewer step", () => {
    const { rerender } = render(
      <ApprovalGate
        rows={[approval({ review_state: "reviewed", reviewed_by: "eyes", reviewed_at: "2026-10-28T09:16:00.000Z" })]}
        onResolve={resolved()}
      />,
    );
    expect(screen.getByTestId("gate-review")).toHaveTextContent("Reviewed by eyes · 09:16Z");
    // A read from before readers were recorded: the time, and no name —
    // never "the reviewer" standing in for whoever reviews today.
    rerender(
      <ApprovalGate
        rows={[approval({ review_state: "reviewed", reviewed_by: null, reviewed_at: "2026-10-27T10:00:00.000Z" })]}
        onResolve={resolved()}
      />,
    );
    expect(screen.getByTestId("gate-review")).toHaveTextContent("Reviewed · 10:00Z — reader not recorded");
    expect(screen.getByTestId("gate-review")).not.toHaveTextContent(/Reviewed by/);
    rerender(<ApprovalGate rows={[approval({ review_state: "skipped" })]} onResolve={resolved()} />);
    expect(screen.getByTestId("gate-review")).toHaveTextContent("Review skipped");
    rerender(
      <ApprovalGate
        rows={[approval({ review_state: "not_applicable", prompt: "Allow `git push` to `main` in this session's repo?", command_text: "git push origin abc:refs/heads/main" })]}
        onResolve={resolved()}
      />,
    );
    expect(screen.getByTestId("gate-review")).toHaveTextContent("No reviewer step (push gate)");
    rerender(<ApprovalGate rows={[approval({ review_state: "unrecorded" })]} onResolve={resolved()} />);
    expect(screen.queryByTestId("gate-review")).not.toBeInTheDocument();
    rerender(<ApprovalGate rows={[approval()]} onResolve={resolved()} />);
    expect(screen.queryByTestId("gate-review")).not.toBeInTheDocument();
  });

  // Feedback #87 c: the executor's close card said the recap's detail was
  // "yours to act on while the card is up", and the user approved before it
  // had finished. The card now says when its declarer is still mid-turn — a
  // marker, not a hold: the buttons stay live.
  it("marks a close card whose declarer is still acting, and nothing else", () => {
    const close = approval({
      kind: "close",
      command_text: null,
      prompt: "Close session?\nCL learnings delta: written this session.",
    });
    const { rerender } = render(
      <ApprovalGate
        rows={[close]}
        label={(a) => (a === "hands" ? "HANDS · Opus" : a)}
        onResolve={resolved()}
        isActing={(agent) => agent === "hands"}
      />,
    );
    expect(screen.getByTestId("close-still-acting")).toHaveTextContent(
      "HANDS · Opus is still acting on the recap's detail",
    );
    expect(screen.getByRole("button", { name: "Close session" })).not.toBeDisabled();
    // Its turn ended: the marker goes.
    rerender(<ApprovalGate rows={[close]} onResolve={resolved()} isActing={() => false} />);
    expect(screen.queryByTestId("close-still-acting")).not.toBeInTheDocument();
    // A gated command's card never carries it, busy or not.
    rerender(<ApprovalGate rows={[approval()]} onResolve={resolved()} isActing={() => true} />);
    expect(screen.queryByTestId("close-still-acting")).not.toBeInTheDocument();
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
    // The dialog hands over to the viewer instead of staying up behind it
    // (both overlays were `z-50`, and one Escape closed both).
    expect(screen.queryByRole("dialog", { name: "Approval details" })).not.toBeInTheDocument();
  });

  // The close card is not a gated command: Approve closes and archives the
  // session and the typed text becomes the agent's next instruction, so it
  // reads that way. The wire strings stay "Approve"/"Reject".
  it("a close card says what its buttons do, and still sends the wire strings", async () => {
    const onResolve = resolved();
    render(
      <ApprovalGate
        rows={[approval({ kind: "close", command_text: null, prompt: "Close session?\nAll done." })]}
        onResolve={onResolve}
      />,
    );
    expect(screen.getByRole("group", { name: "Close session?" })).toBeInTheDocument();
    expect(screen.getByText("CLOSE SESSION?")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Approve" })).toBeNull();
    fireEvent.change(screen.getByRole("textbox", { name: "Next instruction" }), {
      target: { value: "also bump the cask" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Keep open" }));
    await waitFor(() =>
      expect(onResolve).toHaveBeenCalledWith("c-1", "Reject — also bump the cask", false),
    );
    fireEvent.click(screen.getByRole("button", { name: "Close session" }));
    await waitFor(() => expect(onResolve).toHaveBeenCalledWith("c-1", "Approve", false));
  });

  // A close card's prompt is the recap (up to ~1,200 chars): clamped with a
  // toggle, never truncated — the full text stays in the DOM.
  it("clamps a long request behind a toggle and keeps the whole text", () => {
    const long = Array.from({ length: 12 }, (_, i) => `- item ${i}: done and verified`).join("\n");
    render(<ApprovalGate rows={[approval({ kind: "close", command_text: null, prompt: long })]} onResolve={resolved()} />);
    const toggle = screen.getByRole("button", { name: "show the full request" });
    expect(screen.getByText(/item 11: done/)).toBeInTheDocument();
    fireEvent.click(toggle);
    expect(screen.getByRole("button", { name: "show less" })).toBeInTheDocument();
    // A short request has no toggle.
    render(<ApprovalGate rows={[approval({ choice_id: "c-2", prompt: "Run it?" })]} onResolve={resolved()} />);
    expect(screen.getAllByRole("button", { name: /show the full request|show less/ })).toHaveLength(1);
  });

  it("names the requester as ROLE · Model in Details too — never the stored slug (rc3 D10, round 8)", () => {
    // The card printed the resolved label; the Details dialog's "Requested by"
    // still printed `row.agent` — the slug leak the round-7 sweep found.
    render(
      <ApprovalGate
        rows={[approval({ agent: "hands" })]}
        label={(slug) => (slug === "hands" ? "HANDS · Claude Opus 5" : slug)}
        onResolve={resolved()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Details" }));
    const dialog = screen.getByRole("dialog", { name: "Approval details" });
    expect(dialog.textContent).toContain("HANDS · Claude Opus 5");
    // The bare slug reaches neither the card nor the dialog.
    expect(screen.queryByText("hands")).toBeNull();
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

  // Round 11: an agent's `request_approval` may carry its OWN labels (the
  // tool schema suggests one starting "Approve" and one "Deny"). Since the
  // backend now stamps every approval-context row `kind = approval` — the
  // marker that latches AND lifts the ring — such a row lands in this slot,
  // and it must offer the labels the agent offered, not a hardcoded pair: the
  // pick goes back to the agent verbatim, and a label it never sent is a
  // correctness regression, not a cosmetic one.
  it("renders a custom menu's own labels and resolves with the picked label", async () => {
    const onResolve = resolved();
    render(
      <ApprovalGate
        rows={[
          approval({
            kind: "approval",
            options: ["Approve — read only", "Deny with reason"],
            command_text: null,
          }),
        ]}
        onResolve={onResolve}
      />,
    );
    expect(screen.queryByRole("button", { name: "Approve" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Reject" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Deny with reason" }));
    await waitFor(() =>
      expect(onResolve).toHaveBeenCalledWith("c-1", "Deny with reason", false),
    );
  });

  // The other half of the same rule: the canonical menu still resolves with
  // the LITERAL "Approve" — only that string runs a parked command
  // (`gate_verdict` is fail-closed by design). A refactor that sent
  // `options[0]` through would look identical on screen and break every
  // action gate.
  it("keeps the canonical menu resolving with the literal Approve / Reject", async () => {
    const onResolve = resolved();
    render(
      <ApprovalGate rows={[approval({ kind: "approval" })]} onResolve={onResolve} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Reject" }));
    await waitFor(() =>
      expect(onResolve).toHaveBeenCalledWith("c-1", "Reject", false),
    );
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await waitFor(() =>
      expect(onResolve).toHaveBeenLastCalledWith("c-1", "Approve", false),
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

  it("shows WHY a command was gated on the card face, not only under Details", () => {
    // Feedback #29: the Tool Gate's match line and a prior rejection are what
    // the user decides on at a glance — a destructive keyword that is only
    // DATA inside a grep pattern reads as one when the match is shown.
    const cmd = "grep -cF 'bq rm -r -d -f' tasks.md";
    render(
      <ApprovalGate
        rows={[
          approval({
            prompt:
              "Run gated command in this session's repo?\n\n`" +
              cmd +
              "`\n\nmatched Tool-Gate keyword `rm -r` at col 13.\n\n⚠ You REJECTED this identical command at 2026-09-24T01:00:00Z: \"not now\"",
            command_text: cmd,
          }),
        ]}
        onResolve={resolved()}
      />,
    );
    expect(screen.getByText(/matched Tool-Gate keyword `rm -r` at col 13/)).toBeInTheDocument();
    expect(screen.getByText(/You REJECTED this identical command/)).toBeInTheDocument();
    // The command itself is still rendered exactly once.
    expect(screen.getAllByText(cmd)).toHaveLength(1);
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
    // The ask time renders in the user's locale (round 11 — it used to print
    // the raw UTC string); the exact stamp survives as the title.
    expect(dialog).toHaveTextContent(
      new Date("2026-08-14T10:00:00Z").toLocaleString(),
    );
    expect(dialog.querySelector('[title="2026-08-14T10:00:00Z"]')).not.toBeNull();
    expect(dialog).toHaveTextContent("c-1");
    expect(dialog).toHaveTextContent("Approve / Reject");
    expect(dialog).toHaveTextContent("Context the card never shows.");
    expect(dialog).toHaveTextContent("closes the guard gap the suite proved");
    // One Close since the one Dialog (S2f): the title row's.
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("reject reason (1.0.0 Batch 8 T2)", () => {
  it("a typed reason rides the Reject pick; empty stays the plain menu Reject", async () => {
    const onResolve = resolved();
    const { unmount } = render(
      <ApprovalGate rows={[approval()]} onResolve={onResolve} />,
    );
    fireEvent.change(screen.getByLabelText(/rejection reason/i), {
      target: { value: "wrong branch — retarget staging" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^reject$/i }));
    await waitFor(() =>
      expect(onResolve).toHaveBeenCalledWith(
        "c-1",
        "Reject — wrong branch — retarget staging",
        false,
      ),
    );
    unmount();

    const plain = resolved();
    render(<ApprovalGate rows={[approval()]} onResolve={plain} />);
    fireEvent.click(screen.getByRole("button", { name: /^reject$/i }));
    await waitFor(() => expect(plain).toHaveBeenCalledWith("c-1", "Reject", false));
  });
});
