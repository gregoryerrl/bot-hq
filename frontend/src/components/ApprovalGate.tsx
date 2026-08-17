import { useState } from "react";
import { Button } from "./ui/Button";
import { errorMessage } from "../hooks/useInvoke";
import { cn } from "../lib/cn";
import { authorColorClass } from "./authorColor";
import { formatRelative } from "../lib/time";
import { useFocusTrap } from "../hooks/useFocusTrap";
import { useEscapeKey } from "../hooks/useEscapeKey";
import type { TrayRow } from "./HaltBanner";
import { fileArgInCommand } from "./FileViewerDialog";

/**
 * **An approval is not a question, and it must not be parkable (rc3 D33).**
 *
 * Something is synchronously blocked on the answer — a git hook holding a push
 * open, a gated command that has not run. The tray treated it as one more card
 * in a list, so it sat below three answered questions with a Send button of its
 * own, and the user's report was the predictable one: they answered a row, the
 * session did not move, and a second row appeared under the first.
 *
 * So the gate takes the input slot. Three properties, each deliberate:
 *
 * 1. **It replaces the box rather than sitting near it.** There is nothing else
 *    to do in that slot while a command is blocked, and under D33 the box is
 *    locked anyway whenever participants are working.
 * 2. **It is answered on the spot** — Approve / Reject, one click, no Send.
 * 3. **Pause stays reachable.** Pause is the only interrupt in the product, and
 *    a gate is exactly when a user might want it. A modal you cannot escape is
 *    how a harness loses a user's trust.
 *
 * Approvals queue rather than stack: one at a time, oldest first, with a count.
 * A user approving `git push` needs to read that push, not five commands at
 * once.
 */
export function ApprovalGate({
  rows,
  label,
  hues,
  onResolve,
  onCancel,
  onViewFile,
}: {
  /** Pending approvals, oldest first. Never empty — the caller decides to
   *  render this at all. */
  rows: readonly TrayRow[];
  /** slug → what to print for it (rc3 D10/D20). */
  label?: (agent: string) => string;
  /** Label → hue token (rc3 D20). Absent falls back to the label hash. */
  hues?: Record<string, string>;
  /** Resolve one approval. `confirmStale` re-sends a pick the backend held
   *  back because the request had aged. */
  onResolve: (
    choiceId: string,
    picked: string,
    confirmStale?: boolean,
  ) => Promise<{ kind: string; command?: string; asked_at?: string | null }>;
  /** Pause the session — the one interrupt, kept reachable from here. */
  onCancel?: () => Promise<void>;
  /**
   * Open the file a gated command names (`--body-file /tmp/x.md`, a `.md` or
   * image argument) in the full-screen viewer. The tray card had this since
   * the viewer shipped; the gate lost it when rc3 D33 moved approvals into the
   * input slot, and the user approved `gh issue comment … --body-file` bodies
   * they could not see (seven of them on 2026-08-17). Optional only so the
   * component renders in isolation; SessionView always wires it.
   */
  onViewFile?: (path: string) => void;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showDetails, setShowDetails] = useState(false);
  const [stale, setStale] = useState<{
    picked: string;
    command: string;
    askedAt: string | null;
  } | null>(null);

  const row = rows[0];
  if (!row) return null;
  const who = label?.(row.agent) ?? row.agent;
  const more = rows.length - 1;

  const resolve = async (picked: string, confirmStale = false) => {
    if (busy) return;
    setBusy(picked);
    setError(null);
    try {
      const res = await onResolve(row.choice_id, picked, confirmStale);
      // The backend refuses a blind approve on an aged request: the repo may
      // have moved under it, and running the command anyway is the one
      // irreversible mistake this surface can make.
      if (res.kind === "needs_stale_confirm") {
        setStale({
          picked,
          command: res.command ?? row.command_text ?? "",
          askedAt: res.asked_at ?? row.asked_at,
        });
      } else {
        setStale(null);
      }
    } catch (e) {
      // Answering IS the action here. A silent failure would leave the gate up
      // with no signal, and the user pressing Approve again.
      // `errorMessage` — a Tauri rejection is a plain `AppError` object, not an
      // `Error`, and `String(e)` rendered it as "[object Object]" (round 9).
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div
      role="group"
      aria-label="Approval required"
      className="border-t border-primary/40 bg-surface-container-low p-3"
    >
      <div className="flex items-baseline gap-2">
        <span className="font-label-caps text-label-caps text-primary">
          ⛔ APPROVAL
        </span>
        <span className={cn("text-sm font-semibold", authorColorClass(who, hues))}>
          {who}
        </span>
        <span className="text-xs text-on-surface-variant">
          is blocked until you answer
        </span>
        <button
          type="button"
          onClick={() => setShowDetails(true)}
          className="ml-auto text-xs text-primary underline underline-offset-2"
          title="Everything about this gate — the full request, the exact command, who asked and when"
        >
          Details
        </button>
        {more > 0 && (
          <span className="text-xs text-on-surface-variant">
            1 of {rows.length} · {more} more after this
          </span>
        )}
      </div>

      <p className="mt-1.5 text-sm text-on-surface">{gatePrompt(row)}</p>

      {/* The command verbatim. Never truncated — approving something you were
          shown half of is not approval. It scrolls VERTICALLY; the page does
          not, and neither does this.

          `overflow-auto` was a horizontal scroller on the one surface where a
          half-read line is the whole risk: a long command ran off the right edge
          and the user approved what they could see. The house rule (no
          horizontal scrolling, ever) is the pair `overflow-y-auto
          overflow-x-hidden` — a bare `overflow-y-auto` is not enough, since CSS
          computes an unspecified `overflow-x` to `auto` when the other axis is
          non-visible — plus wrapping, so the long line goes DOWN into a box that
          already scrolls. `break-all`, not `break-words`: a command is one
          unbroken token far more often than prose is. */}
      {row.command_text && (
        <pre className="mt-1.5 max-h-32 overflow-y-auto overflow-x-hidden whitespace-pre-wrap break-all rounded border border-outline-variant bg-surface-container px-2 py-1.5 font-mono text-xs text-on-surface">
          {row.command_text}
        </pre>
      )}
      <ViewFileButton command={row.command_text} onViewFile={onViewFile} />

      {stale ? (
        <div className="mt-2 rounded border border-error/50 bg-error-container/30 p-2">
          <p className="text-xs text-on-surface">
            Requested {stale.askedAt ? formatRelative(stale.askedAt) : "earlier"}
            . The repo may have moved since — confirm you still want this to run.
          </p>
          <div className="mt-2 flex gap-2">
            <Button
              type="button"
              variant="danger"
              disabled={!!busy}
              onClick={() => void resolve(stale.picked, true)}
            >
              Run it anyway
            </Button>
            <Button
              type="button"
              variant="secondary"
              disabled={!!busy}
              onClick={() => setStale(null)}
            >
              Cancel
            </Button>
          </div>
        </div>
      ) : (
        <div className="mt-2 flex items-center gap-2">
          <Button
            type="button"
            variant="primary"
            disabled={!!busy}
            onClick={() => void resolve("Approve")}
          >
            {busy === "Approve" ? "Approving…" : "Approve"}
          </Button>
          <Button
            type="button"
            variant="secondary"
            disabled={!!busy}
            onClick={() => void resolve("Reject")}
          >
            {busy === "Reject" ? "Rejecting…" : "Reject"}
          </Button>
          {onCancel && (
            <Button
              type="button"
              variant="danger"
              className="ml-auto"
              disabled={!!busy}
              onClick={() => void onCancel()}
              title="Pause the agents — the one interrupt. The gate stays until you answer it."
            >
              Pause
            </Button>
          )}
        </div>
      )}

      {error && (
        <p role="alert" className="mt-1.5 text-xs text-error">
          {error}
        </p>
      )}
      {showDetails && (
        <GateDetailsDialog
          row={row}
          who={who}
          hues={hues}
          onClose={() => setShowDetails(false)}
          onViewFile={onViewFile}
        />
      )}
    </div>
  );
}

/**
 * Everything about the gate, in one place (vision.md: "Full transparency.
 * Every bit of information the agents see is visible to the user").
 *
 * The gate card shows the prompt's FIRST line and a height-capped command;
 * a `gh pr create --body '…'` approval carries its whole PR body inside
 * that command, cramped into a 8-rem scroll box. The old gate surfaced such
 * payloads as clickable files (view `pr-body.md`); this is that affordance
 * rebuilt for the new gate: the full request text, the exact command with
 * room to read it, and every recorded field — who asked, when, kind, id,
 * options — so nothing about what's being approved is off screen.
 */
function GateDetailsDialog({
  row,
  who,
  hues,
  onClose,
  onViewFile,
}: {
  row: TrayRow;
  /** The requester as `ROLE · Model` (rc3 D10) — never the stored slug. */
  who: string;
  hues?: Record<string, string>;
  onClose: () => void;
  onViewFile?: (path: string) => void;
}) {
  const trapRef = useFocusTrap<HTMLDivElement>(true);
  useEscapeKey(onClose, true);
  return (
    <>
      <div
        className="fixed inset-0 z-40 bg-black/60"
        onClick={onClose}
        aria-hidden
      />
      <div
        ref={trapRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label="Approval details"
        className={cn(
          "fixed left-1/2 top-1/2 z-50 flex max-h-[85vh] w-[min(760px,94vw)]",
          "-translate-x-1/2 -translate-y-1/2 flex-col rounded-lg border",
          "border-outline-variant bg-surface-container p-5 shadow-2xl focus:outline-none",
        )}
      >
        <h2 className="mb-3 font-headline-md text-headline-md text-on-surface">
          Approval details
        </h2>
        <div className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden pr-1">
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
            <dt className="text-on-surface-variant">Requested by</dt>
            <dd className={cn("font-semibold", authorColorClass(who, hues))}>
              {who}
            </dd>
            <dt className="text-on-surface-variant">Asked</dt>
            <dd className="text-on-surface">
              {formatRelative(row.asked_at)}{" "}
              <span className="text-on-surface-variant">({row.asked_at})</span>
            </dd>
            <dt className="text-on-surface-variant">Kind</dt>
            <dd className="text-on-surface">
              {row.command_text ? "gated command (Tool Gate)" : row.kind}
            </dd>
            <dt className="text-on-surface-variant">Options</dt>
            <dd className="text-on-surface">{row.options.join(" / ")}</dd>
            <dt className="text-on-surface-variant">Gate id</dt>
            <dd className="font-mono text-on-surface-variant">{row.choice_id}</dd>
          </dl>

          <p className="mt-3 text-xs font-medium text-on-surface-variant">
            Full request
          </p>
          <p className="mt-1 whitespace-pre-wrap rounded border border-outline-variant bg-surface-container-low px-2 py-1.5 text-sm text-on-surface">
            {row.prompt}
          </p>

          {row.command_text && (
            <>
              <p className="mt-3 text-xs font-medium text-on-surface-variant">
                Exact command — runs verbatim on Approve
              </p>
              {/* Same pair as the box above, and this one matters more: it is
                  the stale-gate confirm, where the user is being asked to
                  re-approve a command against a repo that may have moved. */}
              <pre className="mt-1 overflow-y-auto overflow-x-hidden whitespace-pre-wrap break-all rounded border border-outline-variant bg-surface-container-low px-2 py-1.5 font-mono text-xs text-on-surface">
                {row.command_text}
              </pre>
              <ViewFileButton command={row.command_text} onViewFile={onViewFile} />
            </>
          )}
        </div>
        <div className="mt-4 flex justify-end">
          <Button variant="secondary" onClick={onClose}>
            Close
          </Button>
        </div>
      </div>
    </>
  );
}

/**
 * What to print above the buttons.
 *
 * An action-gate row's prompt is the boilerplate "Run gated command in this
 * session's repo?" followed by the command in a fenced block — and the command
 * gets its own `<pre>` here, so repeating it would show it twice. A push gate
 * has no command and its prompt IS the question ("Allow `git push` to `staging`
 * …"), so that one is printed as written.
 */
function gatePrompt(row: TrayRow): string {
  if (!row.command_text) return row.prompt;
  const firstLine = row.prompt.split("\n", 1)[0]?.trim();
  return firstLine || "Run gated command in this session's repo?";
}

/**
 * "View <file>" for a gated command that names one. A gate that shows only a
 * PATH is a body approved unseen — the exact gap the tray card closed and the
 * D33 gate reopened. Renders nothing when the command names no file or no
 * viewer is wired.
 */
function ViewFileButton({
  command,
  onViewFile,
}: {
  command: string | null | undefined;
  onViewFile?: (path: string) => void;
}) {
  if (!command || !onViewFile) return null;
  const file = fileArgInCommand(command);
  if (!file) return null;
  return (
    <div className="mt-1.5">
      <Button size="sm" variant="ghost" onClick={() => onViewFile(file)}>
        View {file.split("/").pop()}
      </Button>
    </div>
  );
}
