// Checked mirrors of tauri_cmd/participant_changes.rs.
export type ParticipantChangeRequest =
  | { kind: "add"; role_id: number; model_id: string; label: string | null }
  | { kind: "remove"; participant_id: number }
  | { kind: "switch_model"; participant_id: number; model_id: string }
  | { kind: "set_effort"; participant_id: number; effort: string; ultracode: boolean };
export type EffortStatus = {
  available: boolean;
  reason: string | null;
  current: { effort: string; ultracode: boolean } | null;
  ultracode_available: boolean;
};
export type ParticipantChangeView = {
  id: string;
  request: ParticipantChangeRequest;
  status: "pending" | "applying" | "applied" | "failed" | "cancelled";
  error: string | null;
};
export type SessionParticipantControls = {
  revision: number;
  closed: boolean;
  max_participants: number;
  selections: { participant_id: number; model_id: string | null; effort?: EffortStatus | null }[];
  latest_change: ParticipantChangeView | null;
};
