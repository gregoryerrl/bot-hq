import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { errorMessage, useTauriMutation, useTauriQuery } from "../hooks/useInvoke";
import { participantLabel, useSessionParticipants } from "../lib/participants";
import type { ParticipantChangeRequest, ParticipantChangeView, SessionParticipantControls } from "../lib/participantChanges";
import { Button } from "../components/ui/Button";
import { fieldClass } from "../components/ui/field";
import { EFFORT_LEVELS, ULTRACODE, effortChoiceOf, pickToFields } from "../lib/effort";

type SavedRole = { id: number; display_name: string; default_model_id: string | null; archived: boolean };
type SavedModel = { id: string; display_name: string; runtime: string };
type ChangeArgs = { sessionId: string; requestId: string; revision: number; request: ParticipantChangeRequest };

/** Edits are immediate requests; the worker applies them at a completed turn. */
export function SessionRosterEditor({ sessionId }: { sessionId: string }) {
  const qc = useQueryClient();
  const { participants } = useSessionParticipants(sessionId);
  const state = useTauriQuery<SessionParticipantControls>("get_session_participant_controls", { sessionId }, { refetchInterval: 2000 });
  const roles = useTauriQuery<SavedRole[]>("list_roles", { includeArchived: false });
  const models = useTauriQuery<SavedModel[]>("list_models");
  const change = useTauriMutation<ParticipantChangeView, ChangeArgs>("change_session_participant");
  const cancel = useTauriMutation<boolean, { sessionId: string; requestId: string }>("cancel_session_participant_change");
  const [role, setRole] = useState("");
  const [model, setModel] = useState("");
  const [label, setLabel] = useState("");
  const [picks, setPicks] = useState<Record<number, string>>({});
  const [effortPicks, setEffortPicks] = useState<Record<number, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState<ChangeArgs | null>(null);
  const inFlight = useRef(false);
  const current = state.data?.latest_change;
  const pending = current?.status === "pending" || current?.status === "applying";
  const disabled = !state.data || state.data.closed || pending || change.isPending || cancel.isPending;
  const active = participants.filter((p) => p.enabled);
  const activeTurns = active.filter((p) => p.participation_mode === "active").length;

  // Polling also recovers a missed event/reply and a panel reopened mid-change.
  useEffect(() => {
    if (!state.data) return;
    void qc.invalidateQueries({ queryKey: ["list_session_participants", { sessionId }] });
  }, [qc, sessionId, state.data?.revision, current?.status]);
  useEffect(() => {
    if (current?.id === retry?.requestId) { setRetry(null); setError(null); }
  }, [current?.id, retry?.requestId]);

  async function submit(request: ParticipantChangeRequest, again?: ChangeArgs) {
    if (inFlight.current || !state.data) return;
    inFlight.current = true;
    setError(null);
    const same = retry?.revision === state.data.revision && JSON.stringify(retry.request) === JSON.stringify(request);
    const args = again ?? (same ? retry! : { sessionId, requestId: crypto.randomUUID(), revision: state.data.revision, request });
    try {
      const result = await change.mutateAsync(args);
      qc.setQueryData<SessionParticipantControls>(["get_session_participant_controls", { sessionId }], (old) => old && ({ ...old, latest_change: result }));
      setRetry(null);
      if (request.kind === "add") setLabel("");
    } catch (e) {
      setError(errorMessage(e));
      // A lost reply must be retried with the SAME ID, never create an extra invite.
      setRetry(args);
    } finally {
      inFlight.current = false;
      void state.refetch();
    }
  }

  const savedModels = models.data ?? [];
  const options = savedModels.map((m) => <option key={m.id} value={m.id}>{m.display_name} · {m.runtime === "codex" ? "Codex" : "Claude Code"}</option>);
  const problem = error ?? (state.error && errorMessage(state.error)) ?? (roles.error && errorMessage(roles.error)) ?? (models.error && errorMessage(models.error));

  return (
    <section aria-label="Session agents" className="mb-4 space-y-3 rounded border border-outline-variant/40 bg-surface-container/60 px-3 py-3">
      <h3 className="font-label-caps text-label-caps text-on-surface-variant">Agents</h3>
      <p className="font-code-sm text-code-sm text-on-surface-variant">Changes apply after the current turn finishes. Pending changes can be cancelled. Pause interrupts the turn if needed.</p>
      {participants.length > 0 && <>
        <p className="font-label-caps text-label-caps text-on-surface-variant">Participants (turn order)</p>
        <ol className="space-y-3 font-code-sm text-code-sm text-on-surface-variant">
          {participants.map((p) => {
            const name = participantLabel(p);
            const selected = state.data?.selections.find((s) => s.participant_id === p.id)?.model_id ?? "";
            const pick = picks[p.id] ?? selected;
            const last = p.participation_mode === "active" && activeTurns <= 1;
            const effort = state.data?.selections.find((s) => s.participant_id === p.id)?.effort;
            const effective = effortChoiceOf(effort?.current ?? undefined) ?? "";
            const effortPick = effortPicks[p.id] ?? effective;
            return <li key={p.id} className="space-y-1">
              <p><span className="text-on-surface">{name}</span> · {p.participation_mode}{!p.enabled && " · disabled"}</p>
              {p.enabled && <div className="flex flex-wrap items-center gap-2">
                <select aria-label={`Model for ${name}`} className={`${fieldClass} min-w-0 flex-1`} value={pick} disabled={disabled} onChange={(e) => setPicks({ ...picks, [p.id]: e.target.value })}>
                  {!savedModels.some((m) => m.id === pick) && <option value={pick}>{p.model_display_name ?? "Choose saved model"}</option>}
                  {options}
                </select>
                <Button size="sm" disabled={disabled || !pick || pick === selected} onClick={() => void submit({ kind: "switch_model", participant_id: p.id, model_id: pick })}>Change model</Button>
                <Button size="sm" tone="error" aria-label={`Remove ${name}`} disabled={disabled || last || active.length <= 1} title={last ? "Keep at least one active agent" : "History and pending gates are kept"} onClick={() => void submit({ kind: "remove", participant_id: p.id })}>Remove</Button>
              </div>}
              {p.enabled && <div className="space-y-1">
                <div className="flex flex-wrap items-center gap-2">
                  <label className="flex min-w-0 flex-1 items-center gap-2">Effort
                    <select aria-label={`Effort for ${name}`} className={`${fieldClass} min-w-0 flex-1`} value={effortPick} disabled={disabled || !effort?.available} onChange={(e) => setEffortPicks({ ...effortPicks, [p.id]: e.target.value })}>
                      {!effortPick && <option value="">Unavailable</option>}
                      {EFFORT_LEVELS.map((level) => <option key={level} value={level}>{level}</option>)}
                      {effort?.ultracode_available && <option value={ULTRACODE}>Ultracode</option>}
                    </select>
                  </label>
                  <Button size="sm" aria-label={`Change effort for ${name}`} disabled={disabled || !effort?.available || !effortPick || effortPick === effective} onClick={() => {
                    const pair = pickToFields(effortPick);
                    if (pair.effort) void submit({ kind: "set_effort", participant_id: p.id, effort: pair.effort, ultracode: pair.ultracode === true });
                  }}>Change effort</Button>
                </div>
                <p>{effort?.current && `Current effort: ${effective}. `}{effort?.available ? "Changes keep this conversation and apply after the current turn." : effort?.reason ?? "Start or resume this agent to check live effort support."}</p>
              </div>}
            </li>;
          })}
        </ol>
      </>}
      <p className="font-code-sm text-code-sm text-on-surface-variant">Adding an agent or changing its model starts a fresh context and can consume substantial usage. Bounded session excerpts are loaded; older history remains available to read. Other agents keep their context.</p>
      <fieldset disabled={disabled || active.length >= (state.data?.max_participants ?? 8)} className="space-y-2">
        <legend className="mb-1 font-label-caps text-label-caps text-on-surface-variant">Add agent</legend>
        <label className="block font-code-sm text-code-sm">Role
          <select className={fieldClass} value={role} onChange={(e) => { setRole(e.target.value); setModel(roles.data?.find((r) => r.id === Number(e.target.value))?.default_model_id ?? ""); }}>
            <option value="">Choose saved role</option>
            {(roles.data ?? []).filter((r) => !r.archived).map((r) => <option key={r.id} value={r.id}>{r.display_name}</option>)}
          </select>
        </label>
        <label className="block font-code-sm text-code-sm">Model
          <select className={fieldClass} value={model} onChange={(e) => setModel(e.target.value)}><option value="">Choose saved model</option>{options}</select>
        </label>
        <label className="block font-code-sm text-code-sm">Label (optional)
          <input className={fieldClass} value={label} maxLength={256} onChange={(e) => setLabel(e.target.value)} />
        </label>
        <Button size="sm" disabled={!role || !savedModels.some((m) => m.id === model)} onClick={() => void submit({ kind: "add", role_id: Number(role), model_id: model, label: label.trim() || null })}>Add agent</Button>
      </fieldset>
      {state.data?.closed && <p className="font-code-sm text-code-sm">Reopen this session to change its agents.</p>}
      {active.length >= (state.data?.max_participants ?? 8) && <p className="font-code-sm text-code-sm">This session has reached its limit of {state.data?.max_participants ?? 8} agents.</p>}
      {current && <div role="status" className="space-y-1 font-code-sm text-code-sm">
        {pending && current.request.kind === "set_effort" && <p>Requested effort: {current.request.ultracode ? "Ultracode" : current.request.effort}. Awaiting confirmation.</p>}
        <p>{current.status === "pending" ? "Agent change waiting for the current turn to finish." : current.status === "applying" ? "Applying agent change…" : current.status === "applied" ? "Agent change applied." : current.status === "cancelled" ? "Agent change cancelled." : "Agent change failed."}</p>
        {current.error && <p className="text-error">{current.error}</p>}
        {current.status === "pending" && <Button size="sm" disabled={cancel.isPending} onClick={async () => {
          try {
            const cancelled = await cancel.mutateAsync({ sessionId, requestId: current.id });
            setError(cancelled ? null : "The change has started or finished; refresh its status.");
          } catch (e) { setError(errorMessage(e)); }
          void state.refetch();
        }}>Cancel pending change</Button>}
      </div>}
      {problem && <p role="alert" className="font-code-sm text-code-sm text-error">{problem}</p>}
      {retry && error && <Button size="sm" disabled={change.isPending} onClick={() => void submit(retry.request, retry)}>Retry same request</Button>}
      {problem && <Button size="sm" onClick={() => { void state.refetch(); void roles.refetch(); void models.refetch(); }}>Refresh status</Button>}
    </section>
  );
}
