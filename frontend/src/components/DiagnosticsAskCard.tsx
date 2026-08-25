import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTauriQuery } from "../hooks/useInvoke";
import {
  PRIVACY_URL,
  shouldShowDiagnosticsAsk,
  type TelemetryStatus,
} from "../lib/telemetry";

/**
 * The one-time diagnostics ask. Equal-weight buttons — both answers mark the
 * question asked and the card never returns; Settings → Diagnostics remains
 * the place to change your mind either way.
 */
export function DiagnosticsAskCardView({
  onEnable,
  onDecline,
}: {
  onEnable: () => void;
  onDecline: () => void;
}) {
  return (
    <div className="flex items-center gap-3 border-b border-outline-variant bg-surface-container px-grid-margin py-2 font-code-sm text-code-sm text-on-surface">
      <span className="shrink-0 font-label-caps text-label-caps text-on-surface-variant">
        DIAGNOSTICS
      </span>
      <span className="flex-1 truncate">
        Share anonymous crash reports and usage counts (never code or prompts)
        with the bot-hq author?{" "}
        <button
          type="button"
          onClick={() => void openUrl(PRIVACY_URL)}
          className="underline decoration-outline-variant underline-offset-2 hover:text-primary"
        >
          What is sent
        </button>
      </span>
      <button
        type="button"
        onClick={onEnable}
        className="inline-flex shrink-0 items-center rounded border border-outline-variant px-3 py-1 font-code-sm text-code-sm text-on-surface transition-colors hover:bg-surface-container-high"
      >
        Enable
      </button>
      <button
        type="button"
        onClick={onDecline}
        className="inline-flex shrink-0 items-center rounded border border-outline-variant px-3 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface"
      >
        No thanks
      </button>
    </div>
  );
}

export function DiagnosticsAskCard() {
  const status = useTauriQuery<TelemetryStatus>("get_telemetry_status", {});
  if (!shouldShowDiagnosticsAsk(status.data)) return null;

  const answer = async (enable: boolean) => {
    try {
      if (enable) await invoke("set_telemetry_enabled", { enabled: true });
      await invoke("mark_telemetry_asked");
    } finally {
      void status.refetch();
    }
  };

  return (
    <DiagnosticsAskCardView
      onEnable={() => void answer(true)}
      onDecline={() => void answer(false)}
    />
  );
}
