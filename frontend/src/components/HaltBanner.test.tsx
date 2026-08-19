import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import {
  HaltBanner,
  countdownLabel,
  isApproval,
  isTrayItem,
  type SessionHalt,
  type TrayRow,
} from "./HaltBanner";

const row = (o: Partial<TrayRow> = {}): TrayRow => ({
  id: 1,
  choice_id: "c-1",
  agent: "hands",
  kind: "choice",
  prompt: "Which branch?",
  options: ["main", "staging"],
  status: "pending",
  asked_at: "2026-08-14T10:00:00Z",
  command_text: null,
  ...o,
});

const halt = (o: Partial<SessionHalt> = {}): SessionHalt => ({
  declared_by: "hands",
  reason: "Waiting on the output of `php artisan tinker --execute=…`",
  declared_at: "2026-08-14T10:00:00Z",
  ...o,
});

describe("HaltBanner", () => {
  it("renders nothing when the session is not waiting", () => {
    // No halt slot, no pending questions: a banner that is always present
    // stops being read. Answered rows are history — the tray's job.
    const { container } = render(
      <HaltBanner halt={null} rows={[row({ status: "answered" })]} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("says WHO is waiting and WHAT for, from the session's halt slot", () => {
    // rc3 D35: the halt is SESSION state — "halt should be complete different,
    // and not even remotely close to parkable items in tray. It is now a
    // session channel feature." The banner reads the slot, never tray rows.
    render(
      <HaltBanner
        halt={halt()}
        rows={[]}
        label={(a) => (a === "hands" ? "HANDS · Claude Opus 5" : a)}
      />,
    );
    expect(screen.getByText("HANDS · Claude Opus 5")).toBeInTheDocument();
    expect(screen.getByText(/php artisan tinker/)).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveAccessibleName("Session halted");
    expect(screen.getByRole("status")).toHaveTextContent("HALT");
  });

  it("holds exactly ONE halt — the slot, not a list", () => {
    // The user's original design goal, finally by construction: "in this way
    // there can never be 2 halts parked anymore." The session row holds one
    // slot; a later declaration REPLACES the earlier at the source, so this
    // component cannot even express two. The prop is an object, not an array —
    // this test exists to keep it that way.
    render(<HaltBanner halt={halt({ reason: "the freshest recap" })} rows={[]} />);
    expect(screen.getByText(/the freshest recap/)).toBeInTheDocument();
  });

  it("shows questions as a ONE-LINE tray pointer that never says waiting", () => {
    // `ask_user_choice` is non-blocking by design — the agent parks and
    // carries on, and under D35 a question doesn't even touch the ring. The
    // user, twice over: "shorten the message on top of the input box, a one
    // line 'You have questions in the tray' will do" and "Questions in the
    // tray is not equal to Waiting for you. Tray is asynchronous. HALT =
    // waiting for you." So: one clickable line, no HALT, no waiting
    // language anywhere in it.
    render(<HaltBanner halt={null} rows={[row()]} />);
    const banner = screen.getByRole("status");
    expect(banner).toHaveAccessibleName("Questions in the tray");
    expect(banner).not.toHaveTextContent("HALT");
    expect(banner).not.toHaveTextContent(/waiting/i);
    expect(banner).toHaveTextContent("1 question in the tray — the session keeps working");
  });

  it("the tray pointer is a real button — clicking it calls onOpenTray (both branches)", () => {
    // Round 10: the pointer rendered as a button whose handler no site passed,
    // so it looked clickable and did nothing. SessionView now bumps
    // DocumentPane's open-tray signal through this prop; pin that the click
    // reaches it, on the questions-only line AND under a halt — and that the
    // halted branch does not say "waiting" about the tray either (the rule
    // above holds for both).
    const open = vi.fn();
    const { unmount } = render(
      <HaltBanner halt={null} rows={[row()]} onOpenTray={open} />,
    );
    fireEvent.click(screen.getByRole("button", { name: /question in the tray/i }));
    expect(open).toHaveBeenCalledTimes(1);
    unmount();

    render(<HaltBanner halt={halt()} rows={[row()]} onOpenTray={open} />);
    const pointer = screen.getByRole("button", { name: /question.* in the tray/i });
    expect(pointer).not.toHaveTextContent(/waiting/i);
    fireEvent.click(pointer);
    expect(open).toHaveBeenCalledTimes(2);
  });

  it("renders NOTHING for approvals alone — the gate owns that fact", () => {
    // The gate replaces the input box AND halts the session (D35); a banner
    // narrating it on top would be a second surface for one fact.
    const { container } = render(
      <HaltBanner
        halt={null}
        rows={[
          row({
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
    // Load-bearing beyond this component: the discriminator seeds the ring's
    // gate latch and routes rows between the gate and the tray. `command_text`
    // is set for action-gate rows ALONE — keying on it hid 10 of 31 approvals
    // (every push gate). Both gate kinds ask exactly Approve/Reject; no
    // ordinary question ever has.
    expect(isApproval({ options: ["Approve", "Reject"] })).toBe(true);
    expect(
      isApproval({ options: ["Write the EOD first", "Close #499 first"] }),
    ).toBe(false);
    expect(isApproval({ options: [] })).toBe(false);
  });

  it("reads the backend's kind first, and the exact menu as the fallback (round 8)", () => {
    // Since round 8 the backend writes `kind = "approval"` at insert; rows
    // parked before that carry `kind = "choice"` with the gate menu, and both
    // shapes must land in the gate slot — a gate recognised on one path and
    // missed on another reads as a stuck latch.
    expect(isApproval({ kind: "approval", options: ["Approve", "Reject"] })).toBe(true);
    expect(isApproval({ kind: "choice", options: ["Approve", "Reject"] })).toBe(true);
    expect(isApproval({ kind: "approval", options: [] })).toBe(true);
    expect(isApproval({ kind: "choice", options: ["a", "b"] })).toBe(false);
  });

  it("keeps an agent's request in the tray whatever its menu (round 12)", () => {
    // The user's split: request_approval is tray parkable, approval_gates are
    // session blockers. A `request` row with the canonical pair — the shape the
    // descriptor's convention produces — must NOT take the gate slot (it
    // latches nothing, so a gate card there would be a blocker that blocks
    // nothing: issue #1 inverted). The legacy fallback is for the legacy kind.
    expect(isApproval({ kind: "request", options: ["Approve", "Reject"] })).toBe(false);
    expect(isTrayItem({ kind: "request", options: ["Approve", "Reject"] })).toBe(true);
    expect(
      isApproval({
        kind: "request",
        options: ["Approve — commit it", "Approve, and push too", "Deny — read the diff first", "Deny — change the message"],
      }),
    ).toBe(false);
    expect(
      isTrayItem({ kind: "request", options: ["Approve — commit it", "Deny — wait"] }),
    ).toBe(true);
    // The legacy pre-round-8 shape is still a gate.
    expect(isApproval({ kind: "choice", options: ["Approve", "Reject"] })).toBe(true);
  });

  it("sorts rows into exactly one surface each (rc3 D35)", () => {
    // A halt is the banner (session state), an approval is the gate, a
    // question is the tray — and every tray count goes through isTrayItem, so
    // no badge can say "one item on tray" over a tray with nothing in it.
    expect(isTrayItem({ kind: "halt", options: [] })).toBe(false);
    expect(isTrayItem({ kind: "choice", options: ["Approve", "Reject"] })).toBe(
      false,
    );
    expect(isTrayItem({ kind: "choice", options: ["main", "staging"] })).toBe(
      true,
    );
  });

  it("points at the tray when a structured pick is parked too", () => {
    // "in the tray", never "waiting" — the tray is asynchronous on both
    // branches (round 10 aligned the halted branch with the rule above).
    render(<HaltBanner halt={halt()} rows={[row()]} />);
    expect(
      screen.getByRole("button", { name: /1 question in the tray/i }),
    ).toBeInTheDocument();
  });

  it("counts a TEMPORARY halt down, in the user's own shape (round 12)", () => {
    // The user: "TEMPORARY HALT 00:03:57". A halt with a wake instant shows
    // the label, the countdown, and that the session resumes on its own; an
    // ordinary halt keeps the plain HALT header.
    vi.useFakeTimers();
    try {
      vi.setSystemTime(new Date("2026-08-19T12:00:00.000Z"));
      const { unmount } = render(
        <HaltBanner
          halt={halt({ reason: "CI on PR #531", wake_at: "2026-08-19T12:03:57.000Z" })}
          rows={[]}
        />,
      );
      expect(screen.getByTestId("temporary-halt")).toHaveTextContent("TEMPORARY HALT");
      expect(screen.getByText(/wakes in 03:57/)).toBeInTheDocument();
      expect(screen.getByText(/resumes on its own/)).toBeInTheDocument();
      expect(screen.queryByText(/the session is waiting on you/)).toBeNull();
      unmount();
    } finally {
      vi.useRealTimers();
    }
    // Plain halt: no countdown, the old header.
    render(<HaltBanner halt={halt()} rows={[]} />);
    expect(screen.getByText(/the session is waiting on you/)).toBeInTheDocument();
    expect(screen.queryByTestId("temporary-halt")).toBeNull();
  });

  it("formats the countdown as mm:ss, h:mm:ss past an hour, floored at zero", () => {
    const t0 = Date.parse("2026-08-19T12:00:00Z");
    expect(countdownLabel(t0 + 237_000, t0)).toBe("03:57");
    expect(countdownLabel(t0 + 3_661_000, t0)).toBe("1:01:01");
    expect(countdownLabel(t0 + 900, t0)).toBe("00:00");
    expect(countdownLabel(t0 - 5_000, t0)).toBe("00:00");
  });

  it("points a halted session at the approval that took its input box", () => {
    // Both pending at once is reachable: a participant can declare a halt
    // while another's gated tool call sits unanswered. The gate has the input
    // slot (D33), so "answer here" would point at a textarea that is not on
    // screen.
    render(
      <HaltBanner
        halt={halt({ reason: "needs the tinker output" })}
        rows={[
          row({
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
    // Sized against what participants actually write: 28 halts on record,
    // mean 277 chars, longest 1,166. Rendered whole that is ~15 lines of
    // banner above the box it exists to sit next to.
    const recap =
      "PR #514 is open, CI fully green (ci 3m40s, quality 3m35s, search-e2e 1m4s), " +
      "mergeStateStatus CLEAN, auto-close to #509 confirmed registered via closing " +
      "keyword. Branch re-verified against current origin/main. Nothing further I " +
      "can do until you decide whether to merge or hold for the staging promotion.";
    render(<HaltBanner halt={halt({ reason: recap })} rows={[]} />);
    const text = screen.getByText(/PR #514 is open/);
    // Clamped by CSS, not cut in the DOM — the whole recap stays selectable.
    expect(text).toHaveTextContent("staging promotion");
    expect(text.className).toContain("line-clamp-3");
    fireEvent.click(screen.getByRole("button", { name: /show the full recap/i }));
    expect(screen.getByText(/PR #514 is open/).className).not.toContain(
      "line-clamp-3",
    );
    // …and collapses again.
    fireEvent.click(screen.getByRole("button", { name: /show less/i }));
    expect(screen.getByText(/PR #514 is open/).className).toContain(
      "line-clamp-3",
    );
  });

  it("leaves a short reason alone", () => {
    render(
      <HaltBanner
        halt={halt({ reason: "Waiting on the tinker output." })}
        rows={[]}
      />,
    );
    expect(screen.queryByRole("button", { name: /full recap/i })).toBeNull();
  });

  it("tells the user what clears it", () => {
    // "Sending a message clears the halt" is the semantic (one entry point,
    // D28); saying so turns a stopped session from a mystery into an
    // instruction. RESUMES, not "restarts" — the ring picks up where it was.
    render(<HaltBanner halt={halt()} rows={[]} />);
    expect(
      screen.getByText(/clears this and resumes the session/i),
    ).toBeInTheDocument();
  });
});
