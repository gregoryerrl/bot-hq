/**
 * Pure validation for the ingest route — no Workers APIs, so vitest runs it
 * as plain TS. The endpoint URL ships inside every bot-hq binary, which makes
 * this a public unauthenticated POST target by design: these caps (plus the
 * per-IP throttle in index.ts) are the abuse posture, and D1 is unreadable
 * from outside either way.
 */

export const MAX_BODY_BYTES = 64 * 1024;
export const MAX_EVENTS = 100;
export const MAX_FIELD = 64;
export const MAX_AT = 40;
export const MAX_DATA_JSON = 2048;

export const KINDS = ["app_launch", "panic", "error", "counter"] as const;
export type Kind = (typeof KINDS)[number];

export interface BatchEvent {
  kind: Kind;
  at: string;
  data: Record<string, unknown> | null;
}

export interface Batch {
  install_id: string;
  app_version: string;
  os: string;
  arch: string;
  events: BatchEvent[];
}

export type ValidationResult =
  | { ok: true; batch: Batch }
  | { ok: false; error: string };

function shortString(v: unknown, max: number): v is string {
  return typeof v === "string" && v.length > 0 && v.length <= max;
}

/** UUID-shaped: 36 chars, hyphens where they belong. Not a full RFC check. */
function uuidish(v: unknown): v is string {
  return (
    typeof v === "string" &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(v)
  );
}

export function validateBatch(json: unknown): ValidationResult {
  if (typeof json !== "object" || json === null || Array.isArray(json)) {
    return { ok: false, error: "body must be a JSON object" };
  }
  const b = json as Record<string, unknown>;
  if (!uuidish(b.install_id)) return { ok: false, error: "bad install_id" };
  if (!shortString(b.app_version, MAX_FIELD)) return { ok: false, error: "bad app_version" };
  if (!shortString(b.os, MAX_FIELD)) return { ok: false, error: "bad os" };
  if (!shortString(b.arch, MAX_FIELD)) return { ok: false, error: "bad arch" };
  if (!Array.isArray(b.events) || b.events.length === 0) {
    return { ok: false, error: "events must be a non-empty array" };
  }
  if (b.events.length > MAX_EVENTS) {
    return { ok: false, error: `too many events (max ${MAX_EVENTS})` };
  }
  const events: BatchEvent[] = [];
  for (const raw of b.events) {
    if (typeof raw !== "object" || raw === null) {
      return { ok: false, error: "event must be an object" };
    }
    const e = raw as Record<string, unknown>;
    if (!KINDS.includes(e.kind as Kind)) {
      return { ok: false, error: `unknown kind ${JSON.stringify(e.kind)}` };
    }
    if (!shortString(e.at, MAX_AT)) return { ok: false, error: "bad at" };
    let data: Record<string, unknown> | null = null;
    if (e.data !== undefined && e.data !== null) {
      if (typeof e.data !== "object" || Array.isArray(e.data)) {
        return { ok: false, error: "data must be an object" };
      }
      if (JSON.stringify(e.data).length > MAX_DATA_JSON) {
        return { ok: false, error: "data too large" };
      }
      data = e.data as Record<string, unknown>;
    }
    events.push({ kind: e.kind as Kind, at: e.at, data });
  }
  return {
    ok: true,
    batch: {
      install_id: b.install_id,
      app_version: b.app_version,
      os: b.os,
      arch: b.arch,
      events,
    },
  };
}
