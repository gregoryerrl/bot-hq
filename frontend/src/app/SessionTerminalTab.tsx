import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";
import { useTauriEvent } from "../hooks/useTauriEvent";
import type { TerminalOpenView } from "../lib/bindings";

// ============================================================================
// SessionTerminalTab — the session container's "Terminal" subtab: an xterm.js
// view over the session's PTY (core/terminal.rs). The shell runs in the
// session's working repo — the same tree the agents mutate — so the user can
// watch (and type) alongside agent-driven commands. Mounted on first
// activation and kept mounted after, so the buffer survives tab switches.
//
// Data flow: `terminal_open` returns a base64 scrollback snapshot for replay;
// live output arrives as coalesced `terminal:output` events (base64 chunks);
// keystrokes go out through `terminal_input`; the fit addon reports geometry
// via `terminal_resize`.
// ============================================================================

/** Matches the Industrial-Terminal palette (tailwind.config theme colors). */
const XTERM_THEME = {
  background: "#0b1326", // surface
  foreground: "#dae2fd", // on-surface
  cursor: "#dae2fd",
  selectionBackground: "#2d3449", // surface-container-highest
};

function b64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

export function SessionTerminalTab({
  sessionId,
  active,
}: {
  sessionId: string;
  /** Whether the Terminal subtab is the visible one. A hidden container
   * measures 0×0, so fitting is deferred until (re)activation. */
  active: boolean;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const webglRef = useRef<WebglAddon | null>(null);
  // Events that arrive before the snapshot replay finishes queue here, so
  // live chunks can't render ahead of the history they follow.
  const readyRef = useRef(false);
  const queueRef = useRef<Uint8Array[]>([]);

  const fitAndReport = () => {
    const term = termRef.current;
    const fit = fitRef.current;
    if (!term || !fit) return;
    fit.fit();
    invoke("terminal_resize", {
      sessionId,
      cols: term.cols,
      rows: term.rows,
    }).catch(() => {
      // Shell not up yet (or already gone) — the next open/resize catches up.
    });
  };

  // One terminal instance per (mounted) session tab. Created on mount,
  // disposed on unmount; the PTY itself outlives this component and is
  // replayed via the snapshot on the next mount.
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const term = new Terminal({
      fontFamily: '"JetBrains Mono", ui-monospace, monospace',
      fontSize: 12,
      theme: XTERM_THEME,
      scrollback: 5000,
      cursorBlink: true,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(el);
    termRef.current = term;
    fitRef.current = fit;
    // GPU-accelerated rendering for fast streaming output (agent build logs) —
    // xterm's DOM renderer is the slow path there. Load after open() so the
    // renderer is attached; on WebGL context loss dispose the addon and xterm
    // transparently falls back to the DOM renderer. If WebGL2 is unavailable
    // (older GPU, or the jsdom test env), the try/catch leaves the default DOM
    // renderer in place — identical behavior to before.
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        webgl.dispose();
        webglRef.current = null;
      });
      term.loadAddon(webgl);
      webglRef.current = webgl;
    } catch {
      // WebGL unavailable → DOM renderer (the default). Nothing to do.
    }
    readyRef.current = false;
    queueRef.current = [];

    const dataSub = term.onData((data) => {
      invoke("terminal_input", { sessionId, data }).catch(() => {
        // Dead shell: terminal_open on next activation respawns it.
      });
    });

    let disposed = false;
    invoke<TerminalOpenView>("terminal_open", { sessionId })
      .then((view) => {
        if (disposed) return;
        term.write(b64ToBytes(view.snapshot_b64), () => {
          // Flush any live chunks that raced the replay, in arrival order.
          for (const chunk of queueRef.current) term.write(chunk);
          queueRef.current = [];
          readyRef.current = true;
        });
        fitAndReport();
      })
      .catch((e) => {
        if (!disposed) {
          term.writeln(`[failed to open terminal: ${String(e)}]`);
        }
      });

    return () => {
      disposed = true;
      dataSub.dispose();
      webglRef.current?.dispose();
      webglRef.current = null;
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  useTauriEvent<{ session_id: string; data: string; seq: number }>(
    "terminal:output",
    (payload) => {
      if (payload.session_id !== sessionId) return;
      const bytes = b64ToBytes(payload.data);
      if (!readyRef.current) {
        queueRef.current.push(bytes);
      } else {
        termRef.current?.write(bytes);
      }
    },
    [sessionId],
  );

  // Re-fit when the tab becomes visible (hidden → active) and when the
  // container resizes while visible (split drag, window resize).
  useEffect(() => {
    if (!active) return;
    fitAndReport();
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => fitAndReport());
    ro.observe(el);
    return () => ro.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, sessionId]);

  return (
    <div
      ref={containerRef}
      aria-label="Session terminal"
      className="h-full w-full bg-surface px-2 pt-2"
    />
  );
}
