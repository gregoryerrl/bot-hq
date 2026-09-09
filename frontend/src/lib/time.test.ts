import { describe, it, expect } from "vitest";
import { parseUtcMs, formatRelative, formatTimestamp } from "./time";

describe("parseUtcMs", () => {
  const instant = Date.UTC(2026, 5, 3, 7, 40, 0); // 2026-06-03T07:40:00Z

  it("treats a zone-less SQLite string as UTC (the staleness-bug fix)", () => {
    // Runner-TZ-independent: must resolve to the UTC instant, never local.
    expect(parseUtcMs("2026-06-03 07:40:00")).toBe(instant);
  });

  it("parses an explicit-Z RFC3339 string to the same instant", () => {
    expect(parseUtcMs("2026-06-03T07:40:00Z")).toBe(instant);
  });

  it("zone-less and zoned forms of the same instant are equal", () => {
    expect(parseUtcMs("2026-06-03 07:40:00")).toBe(
      parseUtcMs("2026-06-03T07:40:00Z"),
    );
  });

  it("respects an explicit +00:00 offset", () => {
    expect(parseUtcMs("2026-06-03T07:40:00+00:00")).toBe(instant);
  });

  it("respects a non-UTC offset", () => {
    // 15:40+08:00 is the SAME instant as 07:40Z — not 8h apart.
    expect(parseUtcMs("2026-06-03T15:40:00+08:00")).toBe(instant);
  });

  it("returns NaN for empty input", () => {
    expect(Number.isNaN(parseUtcMs(""))).toBe(true);
  });
});

describe("formatRelative", () => {
  it("reads a fresh zone-less UTC timestamp as recent, not hours-stale", () => {
    // Build a zone-less 'now' from UTC fields — the exact shape SQLite emitted.
    const d = new Date();
    const p = (n: number) => String(n).padStart(2, "0");
    const zoneless = `${d.getUTCFullYear()}-${p(d.getUTCMonth() + 1)}-${p(
      d.getUTCDate(),
    )} ${p(d.getUTCHours())}:${p(d.getUTCMinutes())}:${p(d.getUTCSeconds())}`;
    expect(formatRelative(zoneless)).toMatch(/^(just now|\d+s ago)$/);
  });

  it("formats older spans in minutes/hours/days", () => {
    const ago = (ms: number) => new Date(Date.now() - ms).toISOString();
    expect(formatRelative(ago(5 * 60_000))).toBe("5m ago");
    expect(formatRelative(ago(3 * 3_600_000))).toBe("3h ago");
    expect(formatRelative(ago(2 * 86_400_000))).toBe("2d ago");
  });

  it("returns empty string for empty input", () => {
    expect(formatRelative("")).toBe("");
  });
});

describe("formatTimestamp", () => {
  it("is zone-safe and renders a non-empty absolute label", () => {
    expect(formatTimestamp("2026-06-03 07:40:00")).not.toBe("");
    expect(formatTimestamp("2026-06-03 07:40:00")).toBe(
      formatTimestamp("2026-06-03T07:40:00Z"),
    );
  });
});
