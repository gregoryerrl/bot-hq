import { useCallback, useEffect, useRef } from "react";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { useTauriEvent } from "./useTauriEvent";
import { commands } from "../lib/bindings";
import {
  BURST_WINDOW_MS,
  nextUrgentDelay,
  osNotificationsEnabled,
  planFlush,
  urgentToast,
  type EscalationEvent,
  type UrgentGate,
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
 *
 * **An URGENT gate is the exception to all of that** (feedback #52, the
 * user's desktop tier): `session:gate_urgent` fires a toast at once, focused
 * or not, outside the queue and its cooldown, and re-fires every
 * `URGENT_RENOTIFY_MS` until the gate is answered (`session:choice_resolved`)
 * — at most `URGENT_RENOTIFY_MAX` repeats. At mount the pending tray is read
 * once, so a relaunch with an urgent gate still parked notifies again. The
 * Settings Off toggle still silences it: the user asked for no OS
 * notifications at all, and the red tile and card remain.
 */
export function useOsNotifications(): void {
  const queue = useRef<EscalationEvent[]>([]);
  const lastFired = useRef<Record<string, number>>({});
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  /** choice_id → the re-notification timer and how many repeats went out. */
  const urgent = useRef<Map<string, { timer: ReturnType<typeof setTimeout>; repeats: number }>>(
    new Map(),
  );

  const send = useCallback(async (toasts: { title: string; body: string }[]) => {
    try {
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

  const flush = useCallback(async () => {
    timer.current = null;
    const events = queue.current;
    queue.current = [];
    if (events.length === 0) return;
    // The user came back during the burst window — the in-app surfaces are
    // in front of them; a toast now would announce the visible. Drop the
    // queue WITHOUT stamping cooldowns, so nothing is marked as delivered.
    if (document.hasFocus()) return;
    const { toasts, next } = planFlush(events, lastFired.current, Date.now());
    lastFired.current = next;
    if (toasts.length === 0) return;
    await send(toasts);
  }, [send]);

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

  const cancelUrgent = useCallback((choiceId: string) => {
    const entry = urgent.current.get(choiceId);
    if (!entry) return;
    clearTimeout(entry.timer);
    urgent.current.delete(choiceId);
  }, []);

  /** Notify now, then arm the next repeat (or stop at the cap). */
  const fireUrgent = useCallback(
    (g: UrgentGate, repeats: number) => {
      if (!osNotificationsEnabled()) return;
      void send([urgentToast(g, repeats)]);
      const delay = nextUrgentDelay(repeats);
      const previous = urgent.current.get(g.choiceId);
      if (previous) clearTimeout(previous.timer);
      if (delay === null) {
        urgent.current.delete(g.choiceId);
        return;
      }
      const t = setTimeout(() => fireUrgent(g, repeats + 1), delay);
      urgent.current.set(g.choiceId, { timer: t, repeats: repeats + 1 });
    },
    [send],
  );

  // The hook lives for the app's lifetime in Providers, but a pending timer
  // still must not outlive an unmount (tests, refactors).
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
      for (const { timer: t } of urgent.current.values()) clearTimeout(t);
      urgent.current.clear();
    },
    [],
  );

  // A relaunch with an urgent gate still parked: notify once more and arm.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const res = await commands.listPendingTray();
        if (cancelled || res.status !== "ok") return;
        for (const row of res.data) {
          if (row.status !== "pending" || !row.urgent_reason || urgent.current.has(row.choice_id)) {
            continue;
          }
          fireUrgent(
            {
              choiceId: row.choice_id,
              sessionId: row.session_id,
              agent: row.agent,
              reason: row.urgent_reason,
              command: row.command_text ?? "",
            },
            0,
          );
        }
      } catch {
        /* the tray read is best-effort; the next park's event still arrives */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [fireUrgent]);

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
  const onUrgent = useCallback(
    (p: { session_id: string; choice_id: string; agent: string; reason: string; command: string }) => {
      fireUrgent(
        {
          choiceId: p.choice_id,
          sessionId: p.session_id,
          agent: p.agent,
          reason: p.reason,
          command: p.command,
        },
        0,
      );
    },
    [fireUrgent],
  );
  const onResolved = useCallback(
    (p: { choice_id: string }) => cancelUrgent(p.choice_id),
    [cancelUrgent],
  );

  useTauriEvent("session:pending_choice", onChoice, [onChoice]);
  useTauriEvent("session:awaiting_user", onHalt, [onHalt]);
  useTauriEvent("session:gate_urgent", onUrgent, [onUrgent]);
  useTauriEvent("session:choice_resolved", onResolved, [onResolved]);
}
