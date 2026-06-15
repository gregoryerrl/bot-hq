import { useMemo, useState } from "react";
import { useTauriQuery } from "../hooks/useInvoke";
import type {
  ViolationKind,
  ViolationOutcome,
  ViolationRecord,
} from "../lib/bindings";
import { formatTimestamp } from "../lib/time";
import { cn } from "../lib/cn";

// Human labels for the snake_case wire kinds (mirrors policy/violations.rs).
const KIND_LABELS: Record<ViolationKind, string> = {
  push_gate: "Push gate",
  commit_grep: "Commit grep",
  force_push: "Force push",
  tool_blocklist: "Gated command",
  per_action: "Per-action",
  generic_approval: "Approval",
  policy_mutation: "Policy change",
};

const OUTCOME_CLS: Record<ViolationOutcome, string> = {
  approved: "border-success/40 bg-success/15 text-success",
  denied: "border-error/40 bg-error/15 text-error",
  abandoned: "border-warning/40 bg-warning/15 text-warning",
  detected:
    "border-outline-variant bg-surface-container text-on-surface-variant",
};

/**
 * Settings → Violations: a read-only view of the enforcement audit trail
 * (`.local/violations.jsonl`). Every policy gate that fired — push/force-push
 * approvals, commit-word blocks, gated commands, policy-file changes — newest
 * first, filterable by kind / outcome / session. Click-through to the source
 * chat message is a deferred follow-up.
 */
export function ViolationsPanel() {
  const { data, isLoading, error, refetch, isFetching } =
    useTauriQuery<ViolationRecord[]>("read_violations");
  const [kind, setKind] = useState<ViolationKind | "all">("all");
  const [outcome, setOutcome] = useState<ViolationOutcome | "all">("all");
  const [session, setSession] = useState("");

  const rows = useMemo(() => {
    // The log is append-order (oldest first) — show newest first.
    const all = data ? [...data].reverse() : [];
    const q = session.trim().toLowerCase();
    return all.filter(
      (r) =>
        (kind === "all" || r.kind === kind) &&
        (outcome === "all" || r.outcome === outcome) &&
        (q === "" || r.session_id.toLowerCase().includes(q)),
    );
  }, [data, kind, outcome, session]);

  return (
    <div className="mx-auto h-full max-w-5xl overflow-auto px-6 py-6">
      <div className="mb-4 flex items-start justify-between gap-3">
        <div>
          <h1 className="font-headline-lg text-headline-lg text-on-surface">
            Enforcement Log
          </h1>
          <p className="mt-1 max-w-prose font-body-md text-body-md text-on-surface-variant">
            Every policy gate that fired — push / force-push approvals,
            commit-word blocks, gated commands, and policy-file changes. Read
            from <code>.local/violations.jsonl</code>; the full audit trail,
            newest first.
          </p>
        </div>
        <button
          type="button"
          onClick={() => refetch()}
          className="shrink-0 rounded border border-outline-variant px-2.5 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:text-on-surface"
        >
          {isFetching ? "Refreshing…" : "Refresh"}
        </button>
      </div>

      <div className="mb-4 flex flex-wrap items-center gap-2">
        <select
          value={kind}
          onChange={(e) => setKind(e.target.value as ViolationKind | "all")}
          className="rounded border border-outline-variant bg-surface-container px-2 py-1 font-code-sm text-code-sm text-on-surface"
        >
          <option value="all">All kinds</option>
          {(Object.entries(KIND_LABELS) as [ViolationKind, string][]).map(
            ([k, label]) => (
              <option key={k} value={k}>
                {label}
              </option>
            ),
          )}
        </select>
        <select
          value={outcome}
          onChange={(e) =>
            setOutcome(e.target.value as ViolationOutcome | "all")
          }
          className="rounded border border-outline-variant bg-surface-container px-2 py-1 font-code-sm text-code-sm text-on-surface"
        >
          <option value="all">All outcomes</option>
          <option value="approved">Approved</option>
          <option value="denied">Denied</option>
          <option value="abandoned">Abandoned</option>
          <option value="detected">Detected</option>
        </select>
        <input
          value={session}
          onChange={(e) => setSession(e.target.value)}
          placeholder="Filter by session id…"
          className="min-w-[12rem] flex-1 rounded border border-outline-variant bg-surface-container px-2 py-1 font-code-sm text-code-sm text-on-surface placeholder:text-on-surface-variant"
        />
      </div>

      {error ? (
        <p className="rounded border border-error/40 bg-error-container/20 px-3 py-2 font-code-sm text-code-sm text-on-error-container">
          Failed to read the enforcement log: {error.message}
        </p>
      ) : isLoading ? (
        <div className="space-y-2">
          {[0, 1, 2].map((i) => (
            <div
              key={i}
              className="h-10 animate-pulse rounded border border-outline-variant bg-surface-container"
            />
          ))}
        </div>
      ) : rows.length === 0 ? (
        <p className="rounded-lg border border-dashed border-outline-variant p-8 text-center font-code-sm text-code-sm text-on-surface-variant">
          {data && data.length > 0
            ? "No events match the current filters."
            : "No enforcement events recorded yet."}
        </p>
      ) : (
        <div className="overflow-x-auto rounded-lg border border-outline-variant">
          <table className="w-full border-collapse font-code-sm text-code-sm">
            <thead>
              <tr className="border-b border-outline-variant bg-surface-container text-left text-on-surface-variant">
                <th className="px-3 py-2 font-label-caps text-label-caps">
                  When
                </th>
                <th className="px-3 py-2 font-label-caps text-label-caps">
                  Agent
                </th>
                <th className="px-3 py-2 font-label-caps text-label-caps">
                  Kind
                </th>
                <th className="px-3 py-2 font-label-caps text-label-caps">
                  Outcome
                </th>
                <th className="px-3 py-2 font-label-caps text-label-caps">
                  Action
                </th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r, i) => (
                <tr
                  key={i}
                  className="border-b border-outline-variant/40 align-top last:border-0"
                >
                  <td className="whitespace-nowrap px-3 py-2 text-on-surface-variant">
                    {formatTimestamp(r.ts)}
                  </td>
                  <td className="px-3 py-2 capitalize text-on-surface">
                    {r.agent}
                  </td>
                  <td className="whitespace-nowrap px-3 py-2 text-on-surface-variant">
                    {KIND_LABELS[r.kind] ?? r.kind}
                  </td>
                  <td className="px-3 py-2">
                    <span
                      className={cn(
                        "rounded border px-1.5 py-0.5 font-label-caps text-label-caps capitalize",
                        OUTCOME_CLS[r.outcome],
                      )}
                    >
                      {r.outcome}
                    </span>
                  </td>
                  <td className="px-3 py-2 text-on-surface">
                    <span className="break-all">{r.action}</span>
                    {r.detail ? (
                      <span className="mt-0.5 block break-all text-on-surface-variant">
                        {r.detail}
                      </span>
                    ) : null}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {data && data.length > 0 ? (
        <p className="mt-3 font-code-sm text-code-sm text-on-surface-variant">
          {rows.length} of {data.length} event{data.length === 1 ? "" : "s"}
        </p>
      ) : null}
    </div>
  );
}
