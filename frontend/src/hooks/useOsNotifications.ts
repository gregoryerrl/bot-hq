import { useCallback, useEffect, useRef } from "react";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { useTauriEvent } from "./useTauriEvent";
import {
  BURST_WINDOW_MS,
  osNotificationsEnabled,
  planFlush,
  type EscalationEvent,
} from "../lib/osNotifications";

/**
 * Escalate needs-you moments to OS notifications while the window is
 * unfocused: every tray park (questions, approvals, gated commands — all
 * arrive as `session:pending_choice`) and every halt
 * (`session:awaiting_user`). Focused windows stay silent — the in-app bell,
 * tray and halt banner own that case — and focus is re-checked at flush time,
 * because the burst window is exactly long enough for the user to switch back
 * to bot-hq and be staring at the thing a toast would announce.
 *
 * Events queue for a short burst window, then `planFlush` (pure, tested)
 * applies the dedupe/cooldown/coalesce policy. Permission is normally granted
 * from the Settings toggle (a focused, intentional moment); the lazy request
 * here is the fallback for users who never opened Settings, and a denial
 * simply keeps escalation off until they grant it in the OS.
 */
export function useOsNotifications(): void {
  const queue = useRef<EscalationEvent[]>([]);
  const lastFired = useRef<Record<string, number>>({});
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const flush = useCallback(async () => {
    timer.current = null;
    const events = queue.current;
    queue.current = [];
    try {
      if (events.length === 0) return;
      // The user came back during the burst window — the in-app surfaces are
      // in front of them; a toast now would announce the visible. Drop the
      // queue WITHOUT stamping cooldowns, so nothing is marked as delivered.
      if (document.hasFocus()) return;
      const { toasts, next } = planFlush(events, lastFired.current, Date.now());
      lastFired.current = next;
      if (toasts.length === 0) return;
      let granted = await isPermissionGranted();
      if (!granted) granted = (await requestPermission()) === "granted";
      if (!granted) return;
      for (const t of toasts) sendNotification({ title: t.title, body: t.body });
    } catch (e) {
      // No notification daemon (some Linux setups) or plugin failure —
      // escalation is best-effort by design; the in-app surfaces still hold.
      console.error("os-notification send failed", e);
    }
  }, []);

  const push = useCallback(
    (ev: EscalationEvent) => {
      if (!osNotificationsEnabled()) return;
      if (document.hasFocus()) return;
      queue.current.push(ev);
      if (!timer.current) {
        timer.current = setTimeout(() => void flush(), BURST_WINDOW_MS);
      }
    },
    [flush],
  );

  // The hook lives for the app's lifetime in Providers, but a pending timer
  // still must not outlive an unmount (tests, refactors).
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );

  const onChoice = useCallback(
    (p: { session_id: string; question: string }) => {
      push({ sessionId: p.session_id, kind: "question", snippet: p.question });
    },
    [push],
  );
  const onHalt = useCallback(
    (p: { session_id: string; reason: string }) => {
      push({ sessionId: p.session_id, kind: "halt", snippet: p.reason });
    },
    [push],
  );

  useTauriEvent("session:pending_choice", onChoice, [onChoice]);
  useTauriEvent("session:awaiting_user", onHalt, [onHalt]);
}
