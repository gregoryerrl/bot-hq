import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { HaltBanner, type TrayRow } from "./HaltBanner";

const row = (o: Partial<TrayRow> = {}): TrayRow => ({
  id: 1,
  choice_id: "c-1",
  agent: "hands",
  kind: "halt",
  prompt: "Waiting on the output of `php artisan tinker --execute=…`",
  options: [],
  status: "pending",
  asked_at: "2026-08-14T10:00:00Z",
  command_text: null,
  ...o,
});

describe("HaltBanner", () => {
  it("renders nothing when the session is not waiting", () => {
    // A banner that is always present stops being read. Answered rows are
    // history — the tray's job, not this one's.
    const { container } = render(
      <HaltBanner rows={[row({ status: "answered" })]} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("says WHO is waiting and WHAT for", () => {
    // The agent knows both at the moment it stops. Before rc3 D30 that
    // knowledge went to a tab the user had to go looking in, which is why "why
    // is it stopped?" cost six queries across two tables and a log.
    render(
      <HaltBanner
        rows={[row()]}
        label={(a) => (a === "hands" ? "HANDS · Claude Opus 5" : a)}
      />,
    );
    expect(screen.getByText("HANDS · Claude Opus 5")).toBeInTheDocument();
    expect(screen.getByText(/php artisan tinker/)).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveAccessibleName("Session halted");
  });

  it("gives every blocked participant its own line", () => {
    // Reachable since rc3 D22: a park no longer stops the ring where it stands,
    // so the rotation finishes its lap and a second participant can park before
    // it yields. One banner, N lines — the user's "there can never be 2 halts"
    // as a display invariant rather than a claim about the underlying state.
    render(
      <HaltBanner
        rows={[
          row({ id: 1, agent: "hands", prompt: "needs the tinker output" }),
          row({ id: 2, agent: "eyes", prompt: "needs a decision on #512" }),
        ]}
      />,
    );
    expect(screen.getByText(/needs the tinker output/)).toBeInTheDocument();
    expect(screen.getByText(/needs a decision on #512/)).toBeInTheDocument();
  });

  it("points at the tray when a structured pick is waiting too", () => {
    // The division of labour: this banner is the session's STATE; a tray choice
    // is a click. Both can be live at once, which is the user's own example —
    // "HALT — tasks done, question parked in tray".
    render(
      <HaltBanner
        rows={[row(), row({ id: 2, kind: "choice", prompt: "Which branch?" })]}
      />,
    );
    expect(screen.getByRole("button", { name: /1 question waiting/i })).toBeInTheDocument();
  });

  it("does NOT claim the session halted just because a question is pending", () => {
    // The defect this replaces, reported with a screenshot: the banner read
    // "HALT" over a session where both participants were visibly mid-turn, and
    // the status line underneath correctly said so.
    //
    // `ask_user_choice` is non-blocking BY DESIGN — the agent parks and carries
    // on. Only `halt` / `mark_awaiting_user` is a participant saying it stopped.
    // The user: "parking a question in tray toggles the halt (it should not),
    // its asynchronous."
    render(<HaltBanner rows={[row({ kind: "choice" })]} />);
    const banner = screen.getByRole("status");
    expect(banner).toHaveAccessibleName("Waiting for you");
    expect(banner).not.toHaveTextContent("HALT");
    expect(banner).toHaveTextContent("the session is still working");
  });

  it("says a gated approval is BLOCKING, which a question is not", () => {
    // A gated command is a git hook synchronously waiting on a yes/no — the
    // most time-sensitive row in the tray, and the one in the reported
    // screenshot. Calling it a halt understates it in one direction (the
    // session has not stopped) and overstates it in the other (something IS
    // blocked on the user).
    render(
      <HaltBanner
        rows={[
          row({
            kind: "choice",
            options: ["Approve", "Reject"],
            command_text: "git push origin main",
            prompt: "Run gated command in this session's repo",
          }),
        ]}
      />,
    );
    const banner = screen.getByRole("status");
    expect(banner).not.toHaveTextContent("HALT");
    expect(banner).toHaveTextContent("blocked on your approval");
    expect(
      screen.getByRole("button", { name: /1 approval waiting/i }),
    ).toBeInTheDocument();
  });

  it("counts a PUSH gate as an approval, which has no command_text", () => {
    // The defect in the first discriminator, and it hid a third of all
    // approvals. `command_text` is set by `ask_user_choice_inner` for
    // ToolBlocklist (action-gate) rows ALONE; a parked `request_approval` — the
    // push gate — carries none. Measured across the archive: 10 of 31 approvals
    // were push gates, every one classified as an ordinary question while a
    // pre-push hook blocked on it.
    //
    // Both gate kinds ask exactly Approve/Reject, which is what this now keys
    // on. Mutation check: restore `command_text !== null` and this goes red.
    render(
      <HaltBanner
        rows={[
          row({
            kind: "choice",
            options: ["Approve", "Reject"],
            command_text: null,
            prompt: "Allow `git push` to `staging` in this session's repo?",
          }),
        ]}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "blocked on your approval",
    );
  });

  it("does not mistake an ordinary question for an approval", () => {
    // The other direction: free-form options are a question, whatever they say.
    render(
      <HaltBanner
        rows={[
          row({
            kind: "choice",
            options: ["Write the EOD first", "Close #499 properly first"],
            prompt: "Board's clear apart from the EOD. Next?",
          }),
        ]}
      />,
    );
    const banner = screen.getByRole("status");
    expect(banner).toHaveTextContent("a question is waiting");
    expect(banner).not.toHaveTextContent("approval");
  });

  it("still says HALT when a participant actually yielded", () => {
    render(<HaltBanner rows={[row({ kind: "halt" })]} />);
    const banner = screen.getByRole("status");
    expect(banner).toHaveAccessibleName("Session halted");
    expect(banner).toHaveTextContent("HALT");
    expect(banner).toHaveTextContent("the session is waiting on you");
  });

  it("points a halted session at the approval that took its input box", () => {
    // Both pending at once is reachable: D22 lets the lap finish, so one
    // participant can park an approval while another halts. The gate has the
    // input slot (D33), so "answer here" would point at a textarea that is not
    // on screen.
    render(
      <HaltBanner
        rows={[
          row({ id: 1, kind: "halt", prompt: "needs the tinker output" }),
          row({
            id: 2,
            kind: "choice",
            options: ["Approve", "Reject"],
            command_text: "git push origin main",
          }),
        ]}
      />,
    );
    const banner = screen.getByRole("status");
    expect(banner).toHaveTextContent("Answer the approval below first");
    expect(banner).not.toHaveTextContent("Answering — here or in the tray");
  });

  it("tells the user what clears it", () => {
    // "Sending a message clears the halt" is already the semantic (rc3 D28
    // makes every response path do both halves). Saying so is what turns a
    // stopped session from a mystery into an instruction.
    //
    // RESUMES, not "restarts": the ring picks up where it left off — the
    // rotation, the cursors and the tally all survive. "Restart" reads as
    // starting over, which would make a user hesitate to answer.
    render(<HaltBanner rows={[row()]} />);
    expect(screen.getByText(/clears this and resumes the session/i)).toBeInTheDocument();
  });
});
