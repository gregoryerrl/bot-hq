import { useEffect } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * Subscribe to a Tauri event. Handler runs on every emit; unsubscribes on
 * unmount. The deps array follows React's useEffect semantics — if the
 * handler closes over state, include it in deps OR use a ref.
 */
export function useTauriEvent<T>(
  eventName: string,
  handler: (payload: T) => void,
  deps: React.DependencyList = [],
) {
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;

    listen<T>(eventName, (event) => {
      if (!cancelled) handler(event.payload);
    })
      .then((un) => {
        if (cancelled) {
          un();
        } else {
          unlisten = un;
        }
      })
      .catch((e) => {
        console.error(`useTauriEvent: failed to subscribe to "${eventName}"`, e);
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [eventName, ...deps]);
}
