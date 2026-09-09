import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTauriQuery } from "../hooks/useInvoke";
import type { UpdateInfo } from "../lib/bindings";

const DISMISS_KEY = "bot-hq:update-dismissed-version";

/**
 * Pure show/hide decision for the update banner. Shown only when an update is
 * available AND the available version hasn't already been dismissed — so a
 * dismissed `0.2.0` stays hidden, but a later `0.3.0` shows again.
 */
export function shouldShowUpdateBanner(
  info: UpdateInfo | undefined,
  dismissedVersion: string | null,
): boolean {
  if (!info || !info.update_available) return false;
  return info.latest_version !== dismissedVersion;
}

/**
 * Presentational banner. Prop-driven (no data fetching) so it's trivially
 * testable — the wrapper below supplies the data + callbacks.
 */
export function UpdateBannerView({
  info,
  onDownload,
  onDismiss,
}: {
  info: UpdateInfo;
  onDownload: () => void;
  onDismiss: () => void;
}) {
  return (
    <div className="flex items-center gap-3 border-b border-primary/40 bg-primary/10 px-grid-margin py-2 font-code-sm text-code-sm text-on-surface">
      <span className="shrink-0 font-label-caps text-label-caps text-primary">
        UPDATE
      </span>
      <span className="flex-1 truncate">
        bot-hq <span className="text-primary">{info.latest_version}</span> is
        available — you&rsquo;re on {info.current_version}.
      </span>
      <button
        type="button"
        onClick={onDownload}
        className="inline-flex shrink-0 items-center rounded border border-primary bg-primary px-3 py-1 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed"
      >
        Download
      </button>
      <button
        type="button"
        onClick={onDismiss}
        className="shrink-0 rounded border border-outline-variant px-3 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface"
      >
        Dismiss
      </button>
    </div>
  );
}

function safeStorageGet(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function safeStorageSet(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* storage unavailable (e.g. private mode) — non-fatal, just re-prompt */
  }
}

/**
 * App-wide update banner. Checks GitHub Releases once on mount (shared, cached
 * query — the Settings "Updates" panel reuses the same key) and shows a
 * dismissible "download" bar when a newer release exists. Fails quiet: a
 * network error / no-release / rate-limit just renders nothing.
 */
export function UpdateBanner() {
  const { data } = useTauriQuery<UpdateInfo>(
    "check_for_update",
    {},
    {
      retry: false,
      refetchOnWindowFocus: false,
      // Check at most hourly within a run — once-per-launch in practice, well
      // under GitHub's 60 req/hr unauthenticated limit.
      staleTime: 1000 * 60 * 60,
    },
  );
  const [dismissed, setDismissed] = useState<string | null>(() =>
    safeStorageGet(DISMISS_KEY),
  );

  if (!shouldShowUpdateBanner(data, dismissed) || !data) return null;

  return (
    <UpdateBannerView
      info={data}
      onDownload={() => {
        void openUrl(data.release_url);
      }}
      onDismiss={() => {
        safeStorageSet(DISMISS_KEY, data.latest_version);
        setDismissed(data.latest_version);
      }}
    />
  );
}
