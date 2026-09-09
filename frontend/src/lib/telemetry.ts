/**
 * Hand-mirrored shape of `tauri_cmd/telemetry.rs::TelemetryStatus` — the
 * frontend's types are hand-defined (bindings.ts regenerates only at app
 * launch), so a Rust field change must be mirrored here for tsc to see it.
 */
export interface TelemetryStatus {
  enabled: boolean;
  asked: boolean;
  install_id: string | null;
  endpoint: string;
  queued_bytes: number;
}

/** The first-run card shows exactly until the user answers once. */
export function shouldShowDiagnosticsAsk(
  status: TelemetryStatus | undefined,
): boolean {
  return status !== undefined && !status.asked;
}

/** Where the PRIVACY link points — the repo copy of the shipped policy. */
export const PRIVACY_URL =
  "https://github.com/gregoryerrl/bot-hq/blob/main/PRIVACY.md";
