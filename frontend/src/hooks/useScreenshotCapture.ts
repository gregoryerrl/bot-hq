import { useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { errorMessage } from "./useInvoke";

/**
 * Drives the 📸 "share window" button: capture the bot-hq window for a session
 * and track pending + error state. Shared by SessionView and the Emma overlay,
 * which had identical copies of this handler. On failure `capture` sets a
 * string `error` for the caller's dismissible banner.
 */
export function useScreenshotCapture(sessionId: string) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const capture = useCallback(async () => {
    try {
      setPending(true);
      setError(null);
      await invoke("capture_window_screenshot", { sessionId });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setPending(false);
    }
  }, [sessionId]);

  const dismissError = useCallback(() => setError(null), []);

  return { capture, pending, error, dismissError };
}
