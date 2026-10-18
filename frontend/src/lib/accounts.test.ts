import { describe, expect, it } from "vitest";
import type { AccountMark, ModelView } from "./bindings";
import {
  accountLabelOf,
  billsASubscription,
  formatLimitMark,
  formatWhen,
  markIsLive,
  marksFor,
  normalizeConfigDir,
} from "./accounts";

const OPUS: ModelView = {
  id: "m-opus",
  display_name: "Opus",
  provider: "anthropic",
  model_name: "claude-opus-5-5",
  base_url: null,
  auth_token: null,
  context_window: null,
  cli_settings: null,
  claude_config_dir: null,
  created_at: "2026-10-01T00:00:00Z",
  updated_at: "2026-10-01T00:00:00Z",
} as unknown as ModelView;

const mark = (over: Partial<AccountMark> = {}): AccountMark => ({
  config_dir: "",
  org_id: "org-1",
  model_name: "claude-opus-5-5",
  email: "one@example.com",
  limited_until: null,
  limited_text: "You're out of usage credits.",
  marked_at: "2026-10-18T04:08:00Z",
  ...over,
});

describe("accounts", () => {
  it("normalises a config dir the way the backend keys it", () => {
    expect(normalizeConfigDir(null)).toBe("");
    expect(normalizeConfigDir("  /Users/me/.claude-acct-2/ ")).toBe("/Users/me/.claude-acct-2");
    expect(normalizeConfigDir("C:\\Users\\me\\.claude-acct-2\\")).toBe("C:\\Users\\me\\.claude-acct-2");
  });

  it("names the account: default, the dir's last segment, or the provider for a gateway row", () => {
    expect(billsASubscription(OPUS)).toBe(true);
    expect(accountLabelOf(OPUS)).toBe("default");
    expect(accountLabelOf({ ...OPUS, claude_config_dir: "/Users/me/.claude-acct-2/" })).toBe(".claude-acct-2");
    expect(accountLabelOf({ ...OPUS, claude_config_dir: "C:\\Users\\me\\acct2" })).toBe("acct2");
    expect(accountLabelOf({ ...OPUS, auth_token: "t", provider: "OpenRouter" })).toBe("OpenRouter");
    expect(billsASubscription({ ...OPUS, base_url: "https://x" })).toBe(false);
  });

  it("matches marks by dir AND model, never for a gateway row", () => {
    const m = mark();
    expect(marksFor(OPUS, [m])).toEqual([m]);
    expect(marksFor({ ...OPUS, claude_config_dir: "/x" }, [m])).toEqual([]);
    expect(marksFor({ ...OPUS, model_name: "claude-fable-5-1" }, [m])).toEqual([]);
    expect(marksFor({ ...OPUS, auth_token: "t" }, [m])).toEqual([]);
  });

  it("a mark is live until its reset passes; one with no reset stays live", () => {
    const now = Date.parse("2026-10-18T07:00:00Z");
    expect(markIsLive(mark(), now)).toBe(true);
    expect(markIsLive(mark({ limited_until: "2026-10-18T07:00:01Z" }), now)).toBe(true);
    expect(markIsLive(mark({ limited_until: "2026-10-18T06:59:59Z" }), now)).toBe(false);
  });

  it("formats the line without seconds, with the date only when it is not today", () => {
    // Built from LOCAL date parts: any fixed pair of UTC instants falls on two
    // local days in some zone (06:50Z and 07:00Z straddle midnight at UTC−7),
    // and the month's name is the locale's — so the expectations come from
    // the same locale calls, never from a hard-coded "Oct".
    const now = new Date(2026, 9, 18, 12, 0).getTime();
    const sameDay = new Date(2026, 9, 18, 11, 50);
    const otherDay = new Date(2026, 9, 5, 12, 0);
    const time = (d: Date) => d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
    const date = (d: Date) => d.toLocaleDateString(undefined, { month: "short", day: "numeric" });

    const today = formatWhen(sameDay.toISOString(), now);
    expect(today).toBe(time(sameDay));
    expect(today).not.toMatch(/\d:\d\d:\d\d/);
    expect(formatWhen(otherDay.toISOString(), now)).toBe(`${date(otherDay)}, ${time(otherDay)}`);

    const until = formatLimitMark(mark({ limited_until: sameDay.toISOString() }), now);
    expect(until.headline).toBe(`limited until about ${time(sameDay)}`);
    expect(until.detail).toBe('one@example.com: "You\'re out of usage credits."');
    const since = formatLimitMark(mark({ marked_at: otherDay.toISOString() }), now);
    expect(since.headline).toBe(
      `limited since ${date(otherDay)}, ${time(otherDay)} (no reset time given)`,
    );
  });
});
