// Checked mirrors of tauri_cmd/participant_changes.rs.
export type ParticipantChangeRequest =
  | { kind: "add"; role_id: number; model_id: string; label: string | null }
  | { kind: "remove"; participant_id: number }
  | { kind: "switch_model"; participant_id: number; model_id: string };
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
  selections: { participant_id: number; model_id: string | null }[];
  latest_change: ParticipantChangeView | null;
};
