import { useTauriQuery } from "../hooks/useInvoke";
import type { FindingView } from "../lib/bindings";
import { authorColorClass } from "./authorColor";
import { cn } from "../lib/cn";

/**
 * Inline header banner for the EYES-sign-off gate — renders next to the IPAV
 * phase in the session header, in EYES/rain purple. The salience half of the
 * gate (post-mortem §5.2): the user sees unresolved findings at a glance.
 *
 *  - "⚠ N unresolved EYES finding(s)" while blocking findings are OPEN (Brian
 *    must fix or rebut before committing); bold + "(E re-raised)" when any are
 *    escalated (raise_count ≥ 2).
 *  - "✓ M fixed — awaiting Rain's confirm" once an escalated finding has been
 *    resolved but EYES hasn't `approve_finding`'d it yet (the soft-confirm
 *    close loop).
 *
 * Renders nothing when there's nothing to surface. Refetched live via the
 * `session:findings_changed` event (Providers GlobalEventSync → FINDINGS_KEYS).
 */
export function SessionFindingsBanner({ sessionId }: { sessionId: string }) {
  const { data: findings } = useTauriQuery<FindingView[]>(
    "list_session_findings",
    { sessionId },
    { enabled: !!sessionId },
  );
  if (!findings || findings.length === 0) return null;

  const openBlocking = findings.filter(
    (f) => f.status === "open" && f.severity === "blocking",
  );
  if (openBlocking.length > 0) {
    const escalated = openBlocking.filter((f) => f.raise_count >= 2).length;
    return (
      <>
        <span className="mx-2 text-outline-variant">·</span>
        <span
          className={cn(
            authorColorClass("rain"),
            escalated > 0 && "font-semibold",
          )}
          title={
            escalated > 0
              ? `${escalated} re-raised and still undispositioned — needs your attention`
              : "Unresolved EYES blocking findings — Brian must fix or rebut before committing"
          }
        >
          ⚠ {openBlocking.length} unresolved EYES finding
          {openBlocking.length === 1 ? "" : "s"}
          {escalated > 0 ? ` (${escalated} re-raised)` : ""}
        </span>
      </>
    );
  }

  // No open blocking left, but an escalated finding was resolved and still
  // awaits Rain's confirmation.
  const awaitingConfirm = findings.filter(
    (f) =>
      f.raise_count >= 2 &&
      !f.eyes_approved &&
      f.status !== "open" &&
      f.status !== "stale",
  );
  if (awaitingConfirm.length > 0) {
    return (
      <>
        <span className="mx-2 text-outline-variant">·</span>
        <span
          className={authorColorClass("rain")}
          title="An escalated finding was resolved; awaiting Rain's confirmation (approve_finding)"
        >
          ✓ {awaitingConfirm.length} fixed — awaiting Rain's confirm
        </span>
      </>
    );
  }
  return null;
}
