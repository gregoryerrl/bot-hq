import type { AccountMark, ModelView } from "./bindings";
import { parseUtcMs } from "./time";

/**
 * The Claude account a model row bills, and the limit mark on it (migrations
 * 0090/0091) — the one place the Models list and the New Session dialog
 * read them from. Both surfaces used to carry their own copy of each rule
 * and had drifted (two formatters, six copies of the dir normalisation).
 */

/** A saved config dir as the backend keys it: trimmed, no trailing slash.
 *  `""` is the default `~/.claude`. */
export function normalizeConfigDir(dir: string | null | undefined): string {
  return (dir ?? "").trim().replace(/[\\/]+$/, "");
}

/** A subscription-billed row: no gateway token, no gateway URL — the account
 *  it bills is the Claude config dir (blank = the default `~/.claude`). */
export function billsASubscription(m: Pick<ModelView, "auth_token" | "base_url">): boolean {
  return !(m.auth_token && m.auth_token.length > 0) && !(m.base_url && m.base_url.length > 0);
}

/** How the dialog and the list name the account a row bills: blank dir =
 *  `default`; else the dir's last path segment (`.claude-acct-2`); a gateway
 *  row (its own token or base URL) bills its provider, not a Claude account. */
export function accountLabelOf(
  m: Pick<ModelView, "claude_config_dir" | "auth_token" | "base_url" | "provider">,
): string {
  if (!billsASubscription(m)) return m.provider;
  const dir = normalizeConfigDir(m.claude_config_dir);
  if (!dir) return "default";
  const parts = dir.split(/[\\/]/);
  return parts[parts.length - 1] || dir;
}

/** The marks on the account AND model a row bills (claude.ai limits are per
 *  model: an exhausted Fable says nothing about the same account's Opus). A
 *  gateway row is never marked. */
export function marksFor(
  m: Pick<ModelView, "claude_config_dir" | "auth_token" | "base_url" | "model_name">,
  marks: readonly AccountMark[],
): AccountMark[] {
  if (!billsASubscription(m)) return [];
  const key = normalizeConfigDir(m.claude_config_dir);
  return marks.filter((mark) => mark.config_dir === key && mark.model_name === m.model_name);
}

/** Is the mark's reset still ahead (or unknown)? A passed reset is not a
 *  limit any more, whatever the row says until a turn clears it. */
export function markIsLive(mark: AccountMark, now = Date.now()): boolean {
  if (!mark.limited_until) return true;
  const until = parseUtcMs(mark.limited_until);
  return !Number.isFinite(until) || until > now;
}

/** A local clock time, with the date only when it is not today — the
 *  minute, never the second (`toLocaleString()` printed `2:50:00 PM`). */
export function formatWhen(iso: string, now = Date.now()): string {
  const ms = parseUtcMs(iso);
  if (!Number.isFinite(ms)) return iso;
  const d = new Date(ms);
  const today = new Date(now);
  const sameDay =
    d.getFullYear() === today.getFullYear() &&
    d.getMonth() === today.getMonth() &&
    d.getDate() === today.getDate();
  const time = d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  return sameDay ? time : `${d.toLocaleDateString(undefined, { month: "short", day: "numeric" })}, ${time}`;
}

/**
 * The limit line, in two parts: the headline every surface shows
 * ("limited until about 2:50 PM", or "limited since 12:08 PM, no reset time
 * given") and the detail the Models list adds (the account's email and the
 * CLI's own line). The headline keeps the "limited until about" opening the
 * dialog's tests and the user's eye know.
 */
export function formatLimitMark(
  mark: AccountMark,
  now = Date.now(),
): { headline: string; detail: string } {
  const headline = mark.limited_until
    ? `limited until about ${formatWhen(mark.limited_until, now)}`
    : `limited since ${formatWhen(mark.marked_at, now)} (no reset time given)`;
  const who = mark.email ? `${mark.email}: ` : "";
  return { headline, detail: `${who}"${mark.limited_text}"` };
}
