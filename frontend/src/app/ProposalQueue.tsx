import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery, errorMessage } from "../hooks/useInvoke";
import { diffLines, type ProposalDiffLine } from "../lib/proposalDiff";
import type { ClFileContentView, ClProposalView } from "../lib/bindings";

// ============================================================================
// ProposalQueue — project-scoped CL proposal review docket. Hosted under the
// Context Manager's "Proposals" pill, which carries the section label — so
// this renders content only, no heading of its own. Kept fresh by the
// `cl:proposals_changed` event (Providers invalidates cl_list_proposals).
//
// Conflicted proposals (backend-computed `conflict` field) never dead-end:
// the banner explains the divergence and the approve button becomes the
// explicit resolution, sent as `force: true`.
// ============================================================================

const CONFLICT_COPY: Record<string, { button: string; detail: string }> = {
  exists: {
    button: "Replace existing file",
    detail:
      "This add proposal's file now exists in the CL (another proposal or session created it first). Approving replaces the current content with the proposed body — compare before deciding.",
  },
  missing: {
    button: "Create missing file",
    detail:
      "The file this proposal targets no longer exists. Approving recreates it with the proposed body.",
  },
  stale_base: {
    button: "Approve anyway",
    detail:
      "The file changed after this proposal was filed — its body was written against an older version. Approving discards those later changes; compare before deciding.",
  },
};

type DiffState =
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "ready"; lines: ProposalDiffLine[]; note?: string };

const diffLineClass: Record<ProposalDiffLine["kind"], string> = {
  add: "bg-success/10 text-success",
  remove: "bg-error/10 text-error",
  context: "text-on-surface-variant",
};

export function ProposalQueue({
  project,
  onProjectChanged,
}: {
  project: string;
  onProjectChanged: () => void;
}) {
  const {
    data: proposals = [],
    isFetching,
    error,
    refetch,
  } = useTauriQuery<ClProposalView[]>("cl_list_proposals", {
    project,
    status: "open",
  });
  const [busyUids, setBusyUids] = useState<Set<string>>(() => new Set());
  const [actionError, setActionError] = useState<string | null>(null);
  const [diffs, setDiffs] = useState<Record<string, DiffState | undefined>>({});

  const runProposalAction = async (
    proposal: ClProposalView,
    action: "approve" | "reject",
  ) => {
    setBusyUids((prev) => new Set(prev).add(proposal.proposal_uid));
    setActionError(null);
    try {
      // `force` only accompanies a conflict-labelled button — a plain approve
      // stays a plain approve so the backend re-check keeps its teeth.
      await invoke(
        action === "approve" ? "cl_approve_proposal" : "cl_reject_proposal",
        action === "approve"
          ? {
              proposalUid: proposal.proposal_uid,
              force: proposal.conflict != null,
            }
          : { proposalUid: proposal.proposal_uid },
      );
      await refetch();
      onProjectChanged();
    } catch (e) {
      setActionError(errorMessage(e));
    } finally {
      setBusyUids((prev) => {
        const next = new Set(prev);
        next.delete(proposal.proposal_uid);
        return next;
      });
    }
  };

  const toggleDiff = async (proposal: ClProposalView) => {
    const uid = proposal.proposal_uid;
    if (diffs[uid]) {
      setDiffs((prev) => ({ ...prev, [uid]: undefined }));
      return;
    }
    setDiffs((prev) => ({ ...prev, [uid]: { status: "loading" } }));
    try {
      const file = await invoke<ClFileContentView>("cl_read_file", {
        project,
        filePath: proposal.file_path,
      });
      if (file.binary) {
        setDiffs((prev) => ({
          ...prev,
          [uid]: { status: "error", message: "binary file — no text comparison" },
        }));
        return;
      }
      setDiffs((prev) => ({
        ...prev,
        [uid]: {
          status: "ready",
          lines: diffLines(file.content, proposal.proposed_body),
          note: file.truncated
            ? "current file truncated at 1 MB — comparison is partial"
            : undefined,
        },
      }));
    } catch (e) {
      setDiffs((prev) => ({
        ...prev,
        [uid]: { status: "error", message: errorMessage(e) },
      }));
    }
  };

  const approveLabel = (proposal: ClProposalView): string => {
    if (proposal.conflict && CONFLICT_COPY[proposal.conflict]) {
      return CONFLICT_COPY[proposal.conflict].button;
    }
    return proposal.kind === "delete" ? "Delete file" : "Approve";
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-background">
      {actionError && (
        <p className="flex-shrink-0 border-b border-outline-variant px-4 py-1.5 font-code-sm text-code-sm text-error">
          Proposal action failed: {actionError}{" "}
          <button
            onClick={() => setActionError(null)}
            className="underline hover:text-on-surface"
          >
            dismiss
          </button>
        </p>
      )}

      <div className="min-h-0 flex-1 overflow-auto px-4 py-4">
        {isFetching && proposals.length === 0 ? (
          <p className="font-code-sm text-code-sm text-on-surface-variant">
            Loading proposals…
          </p>
        ) : error ? (
          <p className="font-code-sm text-code-sm text-error">
            Failed to load proposals: {error.message}
          </p>
        ) : proposals.length === 0 ? (
          <div className="flex h-full items-center justify-center text-center">
            <div>
              <p className="font-headline-md text-headline-md text-on-surface-variant">
                No open proposals
              </p>
              <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant/60">
                Agents can file CL edits with cl_propose; they will appear here.
              </p>
            </div>
          </div>
        ) : (
          <div className="space-y-3">
            {proposals.map((proposal) => {
              const busy = busyUids.has(proposal.proposal_uid);
              const conflictCopy = proposal.conflict
                ? CONFLICT_COPY[proposal.conflict]
                : undefined;
              const comparable =
                proposal.kind === "correct" || proposal.conflict === "exists";
              const diff = diffs[proposal.proposal_uid];
              return (
                <article
                  key={proposal.proposal_uid}
                  className="grid grid-cols-[4px_minmax(0,1fr)] overflow-hidden rounded border border-outline-variant bg-surface-container-low"
                >
                  <div
                    className={proposal.conflict ? "bg-warning" : "bg-secondary"}
                    aria-hidden
                  />
                  <div className="min-w-0 p-4">
                    <div className="flex flex-wrap items-start justify-between gap-3">
                      <div className="min-w-0">
                        <p className="truncate font-code-sm text-code-sm text-on-surface">
                          {proposal.file_path}
                        </p>
                        <p className="mt-0.5 font-code-sm text-code-sm text-on-surface-variant">
                          proposed by {proposal.proposed_by}
                        </p>
                      </div>
                      <div className="flex shrink-0 items-center gap-2">
                        {proposal.conflict && (
                          <span className="rounded border border-warning/40 bg-warning/10 px-2 py-0.5 font-label-caps text-label-caps text-warning">
                            {proposal.conflict === "stale_base"
                              ? "FILE CHANGED"
                              : proposal.conflict === "exists"
                                ? "FILE EXISTS"
                                : "FILE MISSING"}
                          </span>
                        )}
                        <span className="rounded border border-secondary/40 bg-secondary/10 px-2 py-0.5 font-label-caps text-label-caps text-secondary">
                          {proposal.kind.toUpperCase()}
                        </span>
                        <span className="rounded border border-outline-variant bg-surface-container-high px-2 py-0.5 font-label-caps text-label-caps text-on-surface-variant">
                          {proposal.status.toUpperCase()}
                        </span>
                      </div>
                    </div>

                    {conflictCopy && (
                      <p className="mt-3 rounded border border-warning/40 bg-warning/10 px-3 py-2 font-code-sm text-code-sm text-warning">
                        {conflictCopy.detail}
                      </p>
                    )}
                    {proposal.open_siblings > 0 && (
                      <p className="mt-3 rounded border border-outline-variant/60 bg-surface-container-lowest px-3 py-2 font-code-sm text-code-sm text-on-surface-variant">
                        {proposal.open_siblings} other open proposal
                        {proposal.open_siblings === 1 ? "" : "s"} target
                        {proposal.open_siblings === 1 ? "s" : ""} this file —
                        approving one makes the others stale.
                      </p>
                    )}

                    <div className="mt-3 grid gap-3 lg:grid-cols-2">
                      <section>
                        <p className="mb-1 font-label-caps text-label-caps text-on-surface-variant">
                          Evidence
                        </p>
                        <p className="rounded border border-outline-variant/60 bg-surface-container-lowest px-3 py-2 font-body-md text-body-md text-on-surface">
                          {proposal.evidence}
                        </p>
                      </section>
                      <section>
                        <p className="mb-1 font-label-caps text-label-caps text-on-surface-variant">
                          Proposed body
                        </p>
                        <pre className="max-h-36 overflow-auto rounded border border-outline-variant/60 bg-surface-container-lowest px-3 py-2 font-code-sm text-code-sm text-on-surface whitespace-pre-wrap">
                          {proposal.proposed_body || "(no body — delete proposal)"}
                        </pre>
                      </section>
                    </div>

                    {proposal.target_excerpt && (
                      <section className="mt-3">
                        <p className="mb-1 font-label-caps text-label-caps text-on-surface-variant">
                          Target excerpt
                        </p>
                        <pre className="max-h-28 overflow-auto rounded border border-outline-variant/60 bg-surface-container-lowest px-3 py-2 font-code-sm text-code-sm text-on-surface-variant whitespace-pre-wrap">
                          {proposal.target_excerpt}
                        </pre>
                      </section>
                    )}

                    {comparable && (
                      <section className="mt-3">
                        <button
                          type="button"
                          onClick={() => toggleDiff(proposal)}
                          className="font-code-sm text-code-sm text-primary underline hover:text-primary-fixed"
                        >
                          {diff ? "Hide comparison" : "Compare with current file"}
                        </button>
                        {diff?.status === "loading" && (
                          <p className="mt-2 font-code-sm text-code-sm text-on-surface-variant">
                            Loading current file…
                          </p>
                        )}
                        {diff?.status === "error" && (
                          <p className="mt-2 font-code-sm text-code-sm text-error">
                            Comparison unavailable: {diff.message}
                          </p>
                        )}
                        {diff?.status === "ready" && (
                          <div className="mt-2">
                            {diff.note && (
                              <p className="mb-1 font-code-sm text-code-sm text-warning">
                                {diff.note}
                              </p>
                            )}
                            <pre className="max-h-64 overflow-auto rounded border border-outline-variant/60 bg-surface-container-lowest font-code-sm text-code-sm">
                              {diff.lines.map((line, i) => (
                                <div
                                  key={i}
                                  className={`px-3 whitespace-pre-wrap ${diffLineClass[line.kind]}`}
                                >
                                  {(line.kind === "add"
                                    ? "+ "
                                    : line.kind === "remove"
                                      ? "- "
                                      : "  ") + line.text}
                                </div>
                              ))}
                            </pre>
                          </div>
                        )}
                      </section>
                    )}

                    {proposal.kind === "delete" ? (
                      <p className="mt-3 rounded border border-error/40 bg-error/10 px-3 py-2 font-code-sm text-code-sm text-error">
                        Approving permanently deletes this file from the CL.
                      </p>
                    ) : proposal.kind === "correct" && !proposal.conflict ? (
                      <p className="mt-3 rounded border border-primary/30 bg-primary/10 px-3 py-2 font-code-sm text-code-sm text-primary">
                        Approving replaces the entire file with the proposed body.
                      </p>
                    ) : null}

                    <div className="mt-4 flex justify-end gap-2">
                      <button
                        type="button"
                        onClick={() => runProposalAction(proposal, "reject")}
                        disabled={busy}
                        aria-label="Reject proposal"
                        className="rounded border border-outline-variant bg-transparent px-3 py-1.5 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface disabled:opacity-50"
                      >
                        {busy ? "Working…" : "Reject"}
                      </button>
                      <button
                        type="button"
                        onClick={() => runProposalAction(proposal, "approve")}
                        disabled={busy}
                        aria-label={approveLabel(proposal)}
                        className={
                          proposal.kind === "delete"
                            ? "rounded border border-error bg-error px-3 py-1.5 font-code-sm text-code-sm text-on-error transition-colors hover:bg-error/80 disabled:cursor-not-allowed disabled:opacity-40"
                            : "rounded border border-primary bg-primary px-3 py-1.5 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:cursor-not-allowed disabled:opacity-40"
                        }
                      >
                        {busy ? "Working…" : approveLabel(proposal)}
                      </button>
                    </div>
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
