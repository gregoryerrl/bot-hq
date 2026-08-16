import { render, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen, type Event, type EventCallback } from "@tauri-apps/api/event";
import { Providers } from "./Providers";
import { useActivityStore } from "./stores/activity";
import { useHealthStore } from "./stores/health";
import { slotKey } from "./lib/participants";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const mockInvoke = vi.mocked(invoke);
const mockListen = vi.mocked(listen);

// ===========================================================================
// M12 — the SLOT-SHAPED wire, at its call site
//
// `busyBySlot` and `seedRuntimeStores` are both pinned in stores/runtime.test.ts.
// Neither pin reaches THIS file, and this file is the only thing that connects
// them to the backend: re-keying the `session:activity` payload inline here to
// the literals `brian` / `rain` left the whole suite green (verified 2026-08-12,
// 291 passed). `tsc` then flagged only the now-unused import — one line a
// re-keying edit would delete with it.
//
// So these tests do not exercise the unpackers. They fire the EVENT and read the
// STORE, which is the only path that fails when the wire between them is cut.
// ===========================================================================

/** Every `session:*` handler `GlobalEventSync` subscribed, by event name. */
const handlers = new Map<string, (payload: unknown) => void>();

function emit(event: string, payload: unknown) {
  const handler = handlers.get(event);
  if (!handler) throw new Error(`nothing subscribed to "${event}"`);
  handler(payload);
}

function renderProviders() {
  return render(
    <Providers>
      <div />
    </Providers>,
  );
}

/** One `get_session_runtime` row; every field the backfill reads is present. */
function runtimeRow(over: Record<string, unknown> = {}) {
  return {
    session_id: "s1",
    activity: "busy",
    slot0_busy: true,
    slot1_busy: false,
    slot0_health: "stalled",
    slot1_health: "dead",
    attention: null,
    working: null,
    ...over,
  };
}

describe("Providers — the slot-shaped runtime wire", () => {
  beforeEach(() => {
    handlers.clear();
    mockInvoke.mockReset();
    mockListen.mockReset();
    mockListen.mockImplementation(
      async (event: string, handler: EventCallback<unknown>) => {
        handlers.set(event, (payload) =>
          handler({ event, id: 0, payload } as Event<unknown>),
        );
        return () => {};
      },
    );
    // `get_session_runtime` is the only command Providers itself calls.
    mockInvoke.mockResolvedValue([]);
    useActivityStore.setState({ bySession: {}, busyBySession: {} });
    useHealthStore.setState({
      bySession: {},
      attentionBySession: {},
    });
  });

  it("lands a live session:activity busy pair under SLOT keys, not agent names", async () => {
    renderProviders();
    await waitFor(() => expect(handlers.has("session:activity")).toBe(true));

    emit("session:activity", {
      session_id: "s1",
      state: "busy",
      slot0_busy: true,
      slot1_busy: false,
    });

    const { bySession, busyBySession } = useActivityStore.getState();
    expect(bySession.s1).toBe("busy");
    // The keys the session view resolves a participant through
    // (`participantRuntime`). A literal `brian` / `rain` key here is what made
    // the turn-status line print an agent name no rc3 roster has.
    expect(busyBySession.s1).toEqual({
      [slotKey(0)]: true,
      [slotKey(1)]: false,
    });
  });

  it("keeps the second slot's flag distinct from the first", async () => {
    // A pair keyed by one slot, or by a constant, still passes an equality check
    // that only ever sees `true, false`.
    renderProviders();
    await waitFor(() => expect(handlers.has("session:activity")).toBe(true));

    emit("session:activity", {
      session_id: "s1",
      state: "busy",
      slot0_busy: false,
      slot1_busy: true,
    });

    expect(useActivityStore.getState().busyBySession.s1).toEqual({
      [slotKey(0)]: false,
      [slotKey(1)]: true,
    });
  });

  it("seeds the stores from get_session_runtime on mount, under the same slot keys", async () => {
    // Bug C's backfill. The live event and the mount snapshot have to key one
    // session the same way, or a restart leaves every health dot blank until the
    // next transition.
    mockInvoke.mockResolvedValue([runtimeRow()]);
    renderProviders();

    await waitFor(() =>
      expect(useHealthStore.getState().bySession.s1).toEqual({
        [slotKey(0)]: "stalled",
        [slotKey(1)]: "dead",
      }),
    );
    expect(useActivityStore.getState().busyBySession.s1).toEqual({
      [slotKey(0)]: true,
      [slotKey(1)]: false,
    });
    expect(mockInvoke).toHaveBeenCalledWith("get_session_runtime");
  });

  it("keys live agent_health by the slug the backend emits", async () => {
    // The OTHER key space: `session:agent_health` carries a participant slug, so
    // it must land untouched — `participantRuntime` reads both spaces and the
    // slug wins.
    renderProviders();
    await waitFor(() => expect(handlers.has("session:agent_health")).toBe(true));

    emit("session:agent_health", {
      session_id: "s1",
      agent: "eyes-2",
      health: "retrying",
    });

    expect(useHealthStore.getState().bySession.s1).toEqual({
      "eyes-2": "retrying",
    });
  });
});
