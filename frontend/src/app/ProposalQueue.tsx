import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery, errorMessage } from "../hooks/useInvoke";
import type { ClProposalView } from "../lib/bindings";

// ============================================================================
// ProposalQueue — project-scoped CL proposal review docket. Hosted under the
// Context Manager's "Proposals" pill, which carries the section label — so
// this renders content only, no heading of its own. Kept fresh by the
// `cl:proposals_changed` event (Providers invalidates cl_list_proposals).
// ============================================================================

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

  const runProposalAction = async (
    proposal: ClProposalView,
    action: "approve" | "reject",
  ) => {
    setBusyUids((prev) => new Set(prev).add(proposal.proposal_uid));
    setActionError(null);
    try {
      await invoke(
        action === "approve" ? "cl_approve_proposal" : "cl_reject_proposal",
        { proposalUid: proposal.proposal_uid },
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
              const approveUnsupported = proposal.kind === "delete";
              const busy = busyUids.has(proposal.proposal_uid);
              return (
                <article
                  key={proposal.proposal_uid}
                  className="grid grid-cols-[4px_minmax(0,1fr)] overflow-hidden rounded border border-outline-variant bg-surface-container-low"
                >
                  <div className="bg-secondary" aria-hidden />
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
                        <span className="rounded border border-secondary/40 bg-secondary/10 px-2 py-0.5 font-label-caps text-label-caps text-secondary">
                          {proposal.kind.toUpperCase()}
                        </span>
                        <span className="rounded border border-outline-variant bg-surface-container-high px-2 py-0.5 font-label-caps text-label-caps text-on-surface-variant">
                          {proposal.status.toUpperCase()}
                        </span>
                      </div>
                    </div>

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

                    {approveUnsupported ? (
                      <p className="mt-3 rounded border border-warning/40 bg-warning/10 px-3 py-2 font-code-sm text-code-sm text-warning">
                        Delete approval is deferred for this MVP. Reject this proposal or leave it open.
                      </p>
                    ) : proposal.kind === "correct" ? (
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
                        disabled={busy || approveUnsupported}
                        aria-label={approveUnsupported ? "Approve unsupported" : "Approve proposal"}
                        className="rounded border border-primary bg-primary px-3 py-1.5 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:cursor-not-allowed disabled:opacity-40"
                      >
                        {approveUnsupported ? "Unsupported" : busy ? "Working…" : "Approve"}
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
