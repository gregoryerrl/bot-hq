import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { HaltBanner, type TrayRow } from "./HaltBanner";

const row = (o: Partial<TrayRow> = {}): TrayRow => ({
  id: 1,
  agent: "hands",
  kind: "halt",
  prompt: "Waiting on the output of `php artisan tinker --execute=…`",
  status: "pending",
  asked_at: "2026-08-14T10:00:00Z",
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

  it("shows for a pending choice even with no halt row", () => {
    render(<HaltBanner rows={[row({ kind: "choice" })]} />);
    const banner = screen.getByRole("status");
    expect(banner).toBeInTheDocument();
    // The HEADER line specifically — the tray link below says something
    // similar, and a loose match here found both.
    expect(banner).toHaveTextContent("a question is waiting in the tray");
  });

  it("tells the user what clears it", () => {
    // "Sending a message clears the halt" is already the semantic (rc3 D28
    // makes every response path do both halves). Saying so is what turns a
    // stopped session from a mystery into an instruction.
    render(<HaltBanner rows={[row()]} />);
    expect(screen.getByText(/clears this and restarts the session/i)).toBeInTheDocument();
  });
});
