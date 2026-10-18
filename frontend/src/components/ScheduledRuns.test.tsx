import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { ScheduledRuns, runsLabel } from "./ScheduledRuns";
import type { TrayRow } from "./HaltBanner";

const row = (o: Partial<TrayRow> = {}): TrayRow => ({
  id: 1,
  choice_id: "c-1",
  agent: "hands",
  kind: "choice",
  prompt: "Run gated command?",
  options: ["Approve", "Reject"],
  status: "answered",
  picked_option: "Approve",
  asked_at: "2026-10-13T01:00:00Z",
  command_text: "gh pr merge 803 --squash\n# second line",
  run_not_before: new Date(Date.now() + 4 * 60_000).toISOString(),
  run_not_after: new Date(Date.now() + 10 * 60_000).toISOString(),
  scheduled: true,
  ...o,
});

// Feedback #66a: an approved command waiting for its window is listed with
// when it runs and a Cancel; nothing renders when nothing is scheduled.
describe("ScheduledRuns", () => {
  it("lists a scheduled gate with its time, command and a working Cancel", async () => {
    const onCancel = vi.fn().mockResolvedValue(undefined);
    render(
      <ScheduledRuns
        rows={[row(), row({ id: 2, choice_id: "c-2", scheduled: false })]}
        label={(a) => (a === "hands" ? "HANDS · Opus" : a)}
        onCancel={onCancel}
      />,
    );
    const strip = screen.getByTestId("scheduled-runs");
    expect(strip).toHaveTextContent("⏱ SCHEDULED");
    expect(strip).toHaveTextContent("HANDS · Opus");
    expect(strip).toHaveTextContent(/runs \d\d:\d\d \(in [34] min\)/);
    expect(strip).toHaveTextContent("gh pr merge 803 --squash");
    expect(strip).not.toHaveTextContent("second line");
    // Taking an approval back asks first: the row's button opens the confirm,
    // and only its "Cancel run" withdraws.
    expect(screen.getAllByRole("button", { name: "Cancel run" })).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "Cancel run" }));
    expect(onCancel).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog", { name: "Cancel this scheduled run?" })).toBeInTheDocument();
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Cancel run" }));
    await waitFor(() => expect(onCancel).toHaveBeenCalledWith("c-1"));
  });

  it("renders nothing when no gate is scheduled", () => {
    const { container } = render(<ScheduledRuns rows={[row({ scheduled: false })]} onCancel={vi.fn()} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("shows a failed cancel's message", async () => {
    render(<ScheduledRuns rows={[row()]} onCancel={vi.fn().mockRejectedValue(new Error("not scheduled any more"))} />);
    fireEvent.click(screen.getByRole("button", { name: "Cancel run" }));
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Cancel run" }));
    await waitFor(() => expect(screen.getByText("not scheduled any more")).toBeInTheDocument());
  });

  it("labels the run time on the local clock and counts down", () => {
    const now = new Date(2026, 9, 13, 9, 14).getTime();
    const at = new Date(2026, 9, 13, 9, 18).toISOString();
    expect(runsLabel(at, now)).toBe("runs 09:18 (in 4 min)");
    expect(runsLabel(new Date(2026, 9, 13, 11, 0).toISOString(), now)).toBe("runs 11:00 (in 1 h 46 min)");
    expect(runsLabel(new Date(2026, 9, 13, 9, 10).toISOString(), now)).toBe("runs now (09:10)");
    expect(runsLabel(null, now)).toBe("runs when its window opens");
  });

  it("keeps the run when the confirm is declined", () => {
    const onCancel = vi.fn();
    render(<ScheduledRuns rows={[row()]} onCancel={onCancel} />);
    fireEvent.click(screen.getByRole("button", { name: "Cancel run" }));
    fireEvent.click(screen.getByRole("button", { name: "Keep it scheduled" }));
    expect(onCancel).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
