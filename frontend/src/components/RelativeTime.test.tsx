import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RelativeTime } from "./RelativeTime";

describe("RelativeTime", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-10-18T07:00:00Z"));
  });
  afterEach(() => vi.useRealTimers());

  it("keeps counting on the shared tick and carries the exact time", () => {
    render(<RelativeTime iso="2026-10-18T06:59:50Z" />);
    const el = screen.getByText("10s ago");
    expect(el.tagName).toBe("TIME");
    expect(el).toHaveAttribute("title", new Date("2026-10-18T06:59:50Z").toLocaleString());
    expect(el).toHaveAttribute("dateTime", "2026-10-18T06:59:50.000Z");
    // Two ticks later the label moved without a parent re-render.
    act(() => vi.advanceTimersByTime(61_000));
    expect(screen.getByText("1m ago")).toBeInTheDocument();
  });

  it("falls back for an unreadable timestamp", () => {
    render(<RelativeTime iso="" fallback="earlier" />);
    expect(screen.getByText("earlier")).not.toHaveAttribute("title");
  });
});
