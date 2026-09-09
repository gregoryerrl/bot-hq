/**
 * bot-hq telemetry ingest. One route: `POST /v1/events` with an opt-in
 * diagnostics batch; everything else is 404. `GET /health` answers 200 so a
 * fresh deploy is checkable from a browser.
 *
 * Abuse posture (the URL ships in every binary): body-size + event-count +
 * field caps in validate.ts, plus a per-IP token bucket below. The bucket is
 * isolate-local — an isolate restart refills it — which is fine: it exists to
 * blunt bursts, while the caps are the real ceiling. D1 has no public read
 * path, so the worst an abuser gets is writing junk rows.
 */

import { MAX_BODY_BYTES, validateBatch } from "./validate";

export interface Env {
  TELEMETRY_DB: D1Database;
}

/** ~10 requests/min sustained per IP per isolate, bursts to 10. */
const BUCKET_CAPACITY = 10;
const REFILL_PER_MS = 10 / 60_000;
const buckets = new Map<string, { tokens: number; last: number }>();

export function takeToken(ip: string, now: number): boolean {
  const b = buckets.get(ip) ?? { tokens: BUCKET_CAPACITY, last: now };
  b.tokens = Math.min(BUCKET_CAPACITY, b.tokens + (now - b.last) * REFILL_PER_MS);
  b.last = now;
  if (b.tokens < 1) {
    buckets.set(ip, b);
    return false;
  }
  b.tokens -= 1;
  buckets.set(ip, b);
  // Unbounded-map guard: forget everyone once we cross 10k IPs in this isolate.
  if (buckets.size > 10_000) buckets.clear();
  return true;
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    if (request.method === "GET" && url.pathname === "/health") {
      return new Response("ok", { status: 200 });
    }
    if (request.method !== "POST" || url.pathname !== "/v1/events") {
      return new Response("not found", { status: 404 });
    }
    const ip = request.headers.get("cf-connecting-ip") ?? "unknown";
    if (!takeToken(ip, Date.now())) {
      return new Response("slow down", { status: 429 });
    }
    const len = Number(request.headers.get("content-length") ?? "0");
    if (len > MAX_BODY_BYTES) {
      return new Response("too large", { status: 413 });
    }
    const text = await request.text();
    if (text.length > MAX_BODY_BYTES) {
      return new Response("too large", { status: 413 });
    }
    let json: unknown;
    try {
      json = JSON.parse(text);
    } catch {
      return new Response("bad json", { status: 400 });
    }
    const v = validateBatch(json);
    if (!v.ok) {
      return new Response(v.error, { status: 400 });
    }
    const { batch } = v;
    const stmt = env.TELEMETRY_DB.prepare(
      "INSERT INTO events (install_id, app_version, os, arch, kind, at, data) VALUES (?, ?, ?, ?, ?, ?, ?)",
    );
    await env.TELEMETRY_DB.batch(
      batch.events.map((e) =>
        stmt.bind(
          batch.install_id,
          batch.app_version,
          batch.os,
          batch.arch,
          e.kind,
          e.at,
          e.data === null ? null : JSON.stringify(e.data),
        ),
      ),
    );
    return new Response(JSON.stringify({ accepted: batch.events.length }), {
      status: 202,
      headers: { "content-type": "application/json" },
    });
  },
};
