import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import {
  BacklogChip,
  minutesSince,
  type ParticipantBacklog,
} from "./BacklogChip";

const row = (
  n: number,
  starving: boolean,
  last: string | null = null,
): ParticipantBacklog => ({
  participant_id: 1,
  slug: "eyes",
  last_delivered_at: last,
  undelivered_peer_texts: n,
  starving,
});

describe("BacklogChip", () => {
  it("obeys the backend's starving flag, never the count — the join direction (EYES A6)", () => {
    // A huge count with starving:false renders NOTHING: the chip carries no
    // threshold of its own, so a chip that re-derived the comparison from the
    // count (a second literal that could drift from the scheduler's constant)
    // goes red here.
    const { container } = render(
      <BacklogChip backlog={row(999, false)} name="EYES" />,
    );
    expect(container.innerHTML).toBe("");
  });

  it("renders nothing with no data at all", () => {
    const { container } = render(<BacklogChip backlog={undefined} name="EYES" />);
    expect(container.innerHTML).toBe("");
  });

  it("renders the count when the backend says starving", () => {
    const { getByText } = render(
      <BacklogChip backlog={row(10, true)} name="EYES" />,
    );
    expect(getByText("10 unread")).toBeTruthy();
  });

  it("tooltip carries the lag and the working summons syntax", () => {
    const now = Date.parse("2026-08-27T05:00:00Z");
    const { getByText } = render(
      <BacklogChip
        backlog={row(24, true, "2026-08-27T04:48:00Z")}
        name="EYES"
        now={now}
      />,
    );
    const title = getByText("24 unread").getAttribute("title") ?? "";
    expect(title).toContain("24 peer messages undelivered");
    expect(title).toContain("12m ago");
    // The one summons that works is the USER's mention (participant mentions
    // are inert by design) — the tooltip teaches it.
    expect(title).toContain("@eyes");
  });
});

describe("minutesSince", () => {
  it("never dealt / unparsable → undefined; otherwise floored at 0", () => {
    const now = Date.parse("2026-08-27T05:00:00Z");
    expect(minutesSince(null, now)).toBeUndefined();
    expect(minutesSince("not a date", now)).toBeUndefined();
    expect(minutesSince("2026-08-27T05:01:00Z", now)).toBe(0);
    expect(minutesSince("2026-08-27T03:00:00Z", now)).toBe(120);
  });
});
