import { useTauriQuery } from "../hooks/useInvoke";
import type { FindingView } from "../lib/bindings";
import { cn } from "../lib/cn";
import { WarnIcon } from "./icons";

/**
 * Inline header banner for the reviewer-sign-off gate — renders next to the
 * IPAV phase in the session header. The salience half of the gate (post-mortem
 * §5.2): the user sees unresolved findings at a glance.
 *
 *  - "⚠ N unresolved finding(s)" while blocking findings are OPEN (they must be
 *    fixed or rebutted before committing); bold + "(E re-raised)" when any are
 *    escalated (raise_count ≥ 2).
 *  - "✓ M fixed — awaiting confirmation" once an escalated finding has been
 *    resolved but nobody has `approve_finding`'d it yet (the soft-confirm close
 *    loop).
 *
 * rc3 D10: it names findings, not participants. Who filed one and who has to
 * answer it is the roster's business, and the banner does not claim to know.
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
          className={cn("text-secondary", escalated > 0 && "font-semibold")}
          title={
            escalated > 0
              ? `${escalated} re-raised and still undispositioned — needs your attention`
              : "Unresolved blocking findings — they must be fixed or rebutted before committing"
          }
        >
          <WarnIcon size={12} className="mr-1 inline-block align-[-2px]" />
          {openBlocking.length} unresolved finding
          {openBlocking.length === 1 ? "" : "s"}
          {escalated > 0 ? ` (${escalated} re-raised)` : ""}
        </span>
      </>
    );
  }

  // No open blocking left, but an escalated finding was resolved and still
  // awaits the reviewer's confirmation.
  const awaitingConfirm = findings.filter(
    (f) =>
      f.raise_count >= 2 &&
      !f.reviewer_approved &&
      f.status !== "open",
  );
  if (awaitingConfirm.length > 0) {
    return (
      <>
        <span className="mx-2 text-outline-variant">·</span>
        <span
          className="text-secondary"
          title="An escalated finding was resolved; awaiting confirmation (approve_finding)"
        >
          ✓ {awaitingConfirm.length} fixed — awaiting confirmation
        </span>
      </>
    );
  }
  return null;
}
