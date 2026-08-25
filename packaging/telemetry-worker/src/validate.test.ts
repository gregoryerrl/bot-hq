import { describe, expect, it } from "vitest";
import { MAX_DATA_JSON, MAX_EVENTS, validateBatch } from "./validate";
import { takeToken } from "./index";

const good = () => ({
  install_id: "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d",
  app_version: "1.0.0",
  os: "macos",
  arch: "aarch64",
  events: [{ kind: "app_launch", at: "2026-08-25T00:00:00Z", data: null }],
});

describe("validateBatch", () => {
  it("accepts a well-formed batch", () => {
    const r = validateBatch(good());
    expect(r.ok).toBe(true);
  });

  it("rejects a non-uuid install id", () => {
    const r = validateBatch({ ...good(), install_id: "hello" });
    expect(r).toMatchObject({ ok: false, error: "bad install_id" });
  });

  it("rejects unknown kinds", () => {
    const b = good();
    b.events[0].kind = "keylog" as never;
    expect(validateBatch(b)).toMatchObject({ ok: false });
  });

  it("rejects empty and oversize event arrays", () => {
    expect(validateBatch({ ...good(), events: [] })).toMatchObject({ ok: false });
    const b = good();
    b.events = Array.from({ length: MAX_EVENTS + 1 }, () => ({
      kind: "counter" as const,
      at: "2026-08-25T00:00:00Z",
      data: null,
    }));
    expect(validateBatch(b)).toMatchObject({ ok: false });
  });

  it("rejects oversize data payloads", () => {
    const b = good();
    b.events[0].data = { blob: "x".repeat(MAX_DATA_JSON) } as never;
    expect(validateBatch(b)).toMatchObject({ ok: false, error: "data too large" });
  });

  it("rejects non-object bodies", () => {
    expect(validateBatch([1, 2])).toMatchObject({ ok: false });
    expect(validateBatch("hi")).toMatchObject({ ok: false });
    expect(validateBatch(null)).toMatchObject({ ok: false });
  });
});

describe("takeToken", () => {
  it("allows a burst up to capacity then throttles, refilling over time", () => {
    const ip = `test-${Math.random()}`;
    let allowed = 0;
    for (let i = 0; i < 15; i++) if (takeToken(ip, 1_000)) allowed++;
    expect(allowed).toBe(10);
    expect(takeToken(ip, 1_000)).toBe(false);
    // A minute later the bucket has refilled.
    expect(takeToken(ip, 61_000)).toBe(true);
  });
});
