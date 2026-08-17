import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { SpawnBadge } from "./SpawnBadge";
import type { ParticipantView } from "../lib/participants";

type Knobs = Pick<
  ParticipantView,
  "effort" | "ultracode" | "effort_at_spawn" | "ultracode_at_spawn" | "spawn_knobs_recorded"
>;

/** A row that inherits everything and was spawned after 0061 — the common case. */
function knobs(over: Partial<Knobs> = {}): Knobs {
  return {
    effort: null,
    ultracode: null,
    effort_at_spawn: null,
    ultracode_at_spawn: null,
    spawn_knobs_recorded: true,
    ...over,
  };
}

describe("SpawnBadge — what a participant was actually spawned with", () => {
  it("says nothing for a row that predates the recording", () => {
    // The distinction this whole flag exists for: unrecorded is UNKNOWN, and a
    // guess is worse than a gap. Byte-identical to the case below except for
    // the flag, which is the point.
    const { container } = render(
      <SpawnBadge participant={knobs({ spawn_knobs_recorded: false })} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("says `default` for a recorded row with no override in force", () => {
    // NOT silence. "Spawned with no override" is a real answer, and it is the
    // answer for 94 of 94 live rows — a badge that went quiet here would close
    // the documented gap for none of them.
    render(<SpawnBadge participant={knobs()} />);
    expect(screen.getByText("default")).toBeInTheDocument();
  });

  it("renders the effective effort", () => {
    render(<SpawnBadge participant={knobs({ effort_at_spawn: "high" })} />);
    expect(screen.getByText("high")).toBeInTheDocument();
  });

  it("renders ultracode, and prefers it if both somehow arrive", () => {
    // The backend reconciliation makes both-at-once unreachable. If it arrives
    // anyway the row contradicts a promise, so show the stronger posture rather
    // than pick the weaker one silently.
    render(
      <SpawnBadge
        participant={knobs({ effort_at_spawn: "max", ultracode_at_spawn: true })}
      />,
    );
    expect(screen.getByText("ultracode")).toBeInTheDocument();
  });

  it("treats a blank effort as absent", () => {
    render(<SpawnBadge participant={knobs({ effort_at_spawn: "   " })} />);
    expect(screen.getByText("default")).toBeInTheDocument();
  });

  describe("the styling says whether it was CHOSEN, which the text cannot", () => {
    it("marks a per-run pick as chosen", () => {
      // Same effective value in both tests below — an inherited `high` and a
      // picked `high` are the same string and a different fact, so the choice
      // columns are what separates them.
      render(
        <SpawnBadge participant={knobs({ effort: "high", effort_at_spawn: "high" })} />,
      );
      expect(screen.getByTitle(/picked for this session/i)).toBeInTheDocument();
    });

    it("marks the same value as inherited when nothing was picked", () => {
      render(<SpawnBadge participant={knobs({ effort: null, effort_at_spawn: "high" })} />);
      expect(screen.getByTitle(/inherited from Claude Config/i)).toBeInTheDocument();
    });
  });
});
