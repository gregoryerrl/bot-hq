import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import {
  HaltBanner,
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

  it("points at the tray when a structured pick is waiting too", () => {
    render(<HaltBanner halt={halt()} rows={[row()]} />);
    expect(
      screen.getByRole("button", { name: /1 question waiting/i }),
    ).toBeInTheDocument();
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
