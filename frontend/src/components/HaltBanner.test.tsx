import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { HaltBanner, isApproval, isTrayItem, type TrayRow } from "./HaltBanner";

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

  it("renders NOTHING for approvals alone — the gate owns that fact", () => {
    // **Changed subject at rc3 D35.** This banner used to narrate a pending
    // approval ("a command is blocked on your approval — the session is still
    // working"), and both halves aged out in one day: the gate now replaces
    // the input box (so a banner above it is a second surface for one fact),
    // and an approval now HALTS the session, so "still working" became false.
    const { container } = render(
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
    expect(container).toBeEmptyDOMElement();
  });

  it("classifies a PUSH gate (no command_text) as an approval, and never a question as one", () => {
    // The discriminator is load-bearing beyond this component (rc3 D35): it
    // seeds the ring's gate latch and routes rows between the gate and the
    // tray. `command_text` is set for action-gate rows ALONE — keying on it
    // hid 10 of 31 approvals (every push gate). Both gate kinds ask exactly
    // Approve/Reject; no ordinary question ever has.
    expect(
      isApproval({ options: ["Approve", "Reject"] }),
    ).toBe(true);
    expect(
      isApproval({ options: ["Write the EOD first", "Close #499 first"] }),
    ).toBe(false);
    expect(isApproval({ options: [] })).toBe(false);
  });

  it("sorts rows into exactly one surface each (rc3 D35)", () => {
    // The user: "halt is not on tray anymore, its a declared state." A halt is
    // the banner, an approval is the gate, a question is the tray — and every
    // tray count goes through isTrayItem, so no badge can say "one item on
    // tray" over a tray with nothing in it (the reported defect).
    expect(isTrayItem({ kind: "halt", options: [] })).toBe(false);
    expect(
      isTrayItem({ kind: "choice", options: ["Approve", "Reject"] }),
    ).toBe(false);
    expect(
      isTrayItem({ kind: "choice", options: ["main", "staging"] }),
    ).toBe(true);
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

  it("clamps a long recap so it cannot push the input box away", () => {
    // Sized against what participants actually write: 28 halts on record, mean
    // 277 chars, longest 1,166. Rendered whole that is ~15 lines of banner
    // above the box — the banner would displace the very thing it exists to sit
    // next to. The user's design was "HALT + [short recap]"; the recaps are not
    // short.
    const recap =
      "PR #514 is open, CI fully green (ci 3m40s, quality 3m35s, search-e2e 1m4s), " +
      "mergeStateStatus CLEAN, auto-close to #509 confirmed registered via closing " +
      "keyword. Branch re-verified against current origin/main. Nothing further I " +
      "can do until you decide whether to merge or hold for the staging promotion.";
    render(<HaltBanner rows={[row({ prompt: recap })]} />);
    const text = screen.getByText(/PR #514 is open/);
    // Clamped by CSS, not cut in the DOM — the whole recap is present and
    // selectable, and one click removes the clamp.
    expect(text).toHaveTextContent("staging promotion");
    expect(text.className).toContain("line-clamp-3");
    fireEvent.click(
      screen.getByRole("button", { name: /show the full recap/i }),
    );
    expect(screen.getByText(/PR #514 is open/).className).not.toContain(
      "line-clamp-3",
    );
  });

  it("leaves a short reason alone", () => {
    // Most halts are short and need no affordance. An expander on a one-liner
    // is clutter on the surface that has to stay readable.
    render(<HaltBanner rows={[row({ prompt: "Waiting on the tinker output." })]} />);
    expect(screen.queryByRole("button", { name: /full recap/i })).toBeNull();
    expect(
      screen.getByText("Waiting on the tinker output.").className,
    ).toContain("line-clamp-3");
  });

  it("expands one participant's recap without expanding the other's", () => {
    const long = "x".repeat(400);
    render(
      <HaltBanner
        rows={[
          row({ id: 1, agent: "hands", prompt: `HANDS ${long}` }),
          row({ id: 2, agent: "eyes", prompt: `EYES ${long}` }),
        ]}
      />,
    );
    const toggles = screen.getAllByRole("button", { name: /full recap/i });
    expect(toggles).toHaveLength(2);
    fireEvent.click(toggles[0]!);
    expect(screen.getByText(/^HANDS x+/).className).not.toContain("line-clamp-3");
    expect(screen.getByText(/^EYES x+/).className).toContain("line-clamp-3");
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
