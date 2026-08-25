import { render, waitFor } from "@testing-library/react";
import { useQueryClient } from "@tanstack/react-query";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen, type Event, type EventCallback } from "@tauri-apps/api/event";
import { Providers } from "./Providers";
import { useActivityStore } from "./stores/activity";
import { useHealthStore } from "./stores/health";
import { slotKey } from "./lib/participants";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: vi.fn(async () => true),
  requestPermission: vi.fn(async () => "granted"),
  sendNotification: vi.fn(),
}));

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

  // issues.md #3: the composer's persisted draft is cleared by SessionView's
  // handler ONLY for the session on screen. A staged message delivered to any
  // other session left `bothq:draft:<sid>` holding the sent text, and the box
  // refilled on return. This is the global half: any session, no view needed.
  it("resyncs the chat history too, not only the event-backed side queries (round 8)", async () => {
    // A lagged burst drops MessagePersisted events; the emitter recovers a
    // skipped row only when a later row in that session arrives, so the tail of
    // the burst stays missing until a remount unless the resync refetches
    // get_session_messages (ChatPane re-seeds the store from it).
    const { RESYNC_KEYS } = await import("./Providers");
    expect(RESYNC_KEYS).toContain("get_session_messages");
    expect(RESYNC_KEYS).toContain("get_staged_response");
    expect(RESYNC_KEYS).toContain("get_session");
    // And the resync handler is subscribed at all.
    renderProviders();
    await waitFor(() => expect(handlers.has("session:resync")).toBe(true));
  });

  it("clears a delivered session's persisted draft even when no view for it is mounted", async () => {
    localStorage.setItem("bothq:draft:s2", "queued while they work");
    localStorage.setItem("bothq:draft:s1", "still being typed");
    renderProviders();
    await waitFor(() => expect(handlers.has("session:stage_delivered")).toBe(true));
    emit("session:stage_delivered", { session_id: "s2" });
    expect(localStorage.getItem("bothq:draft:s2")).toBeNull();
    // Keyed on the payload: another session's draft is untouched.
    expect(localStorage.getItem("bothq:draft:s1")).toBe("still being typed");
    localStorage.removeItem("bothq:draft:s1");
  });

  // Round 13, issues.md "still happening": the draft key was only ONE of four
  // stale artifacts a delivery-to-an-unmounted-view leaves. The cached
  // `get_staged_response` re-marked the box staged on return (rehydrating the
  // DELIVERED text), and the surviving trayStaging picks beside answered rows
  // made SessionView's re-stage effect RESEND the message. This handler is the
  // only one that always hears the delivery, so it clears all of it.
  it("drops the delivered session's staged cache and tray picks, no view mounted", async () => {
    const { useTrayStaging } = await import("./stores/trayStaging");
    useTrayStaging.setState({
      staged: { s2: { "choice-a": "Yes" }, s1: { "choice-b": "No" } },
    });
    let client: import("@tanstack/react-query").QueryClient | null = null;
    function Probe() {
      client = useQueryClient();
      return null;
    }
    render(
      <Providers>
        <Probe />
      </Providers>,
    );
    await waitFor(() => expect(handlers.has("session:stage_delivered")).toBe(true));
    client!.setQueryData(["get_staged_response", { sessionId: "s2" }], {
      text: "already sent",
      picks: [{ choice_id: "choice-a", picked: "Yes" }],
    });

    emit("session:stage_delivered", { session_id: "s2" });

    // The stale response is nulled — nothing left to re-mark the box staged…
    expect(
      client!.getQueryData(["get_staged_response", { sessionId: "s2" }]),
    ).toBeNull();
    // …and the picks are consumed — nothing left for the re-stage effect.
    expect(useTrayStaging.getState().staged["s2"]).toBeUndefined();
    // Another session's staging is untouched.
    expect(useTrayStaging.getState().staged["s1"]).toEqual({ "choice-b": "No" });
    useTrayStaging.setState({ staged: {} });
  });

  it("subscribes the plugin registry events once, for the tab row and the manager alike (round 8)", async () => {
    // Shell and PluginManager each carried their own copies of these three
    // listeners; the global map is the one place, invalidating the one query
    // key both read.
    const { RESYNC_KEYS } = await import("./Providers");
    renderProviders();
    for (const ev of ["plugin:state-changed", "plugin:uninstalled", "plugin:crashed"]) {
      await waitFor(() => expect(handlers.has(ev)).toBe(true));
    }
    expect(RESYNC_KEYS).toContain("list_installed_plugins");
  });

  it("purges a closed session's chat store and draft for ANY session (round 8)", async () => {
    const { useChatStore } = await import("./stores/chat");
    const msg = (id: number, session_id: string) =>
      ({ id, session_id, author: "hands", kind: "text", content: "x", created_at: "2026-08-17T00:00:00Z" }) as never;
    useChatStore.getState().setMessages("s-closed", [msg(1, "s-closed")]);
    useChatStore.getState().setMessages("s-open", [msg(2, "s-open")]);
    localStorage.setItem("bothq:draft:s-closed", "never sent");
    localStorage.setItem("bothq:draft:s-open", "still typing");
    renderProviders();
    await waitFor(() => expect(handlers.has("session:closed")).toBe(true));
    emit("session:closed", { session_id: "s-closed" });
    // Only the mounted SessionView used to do this — a session closed while
    // the user was elsewhere kept its messages resident and its draft key forever.
    expect(useChatStore.getState().messages["s-closed"]).toBeUndefined();
    expect(localStorage.getItem("bothq:draft:s-closed")).toBeNull();
    // Keyed on the payload: the other session is untouched.
    expect(useChatStore.getState().messages["s-open"]).toHaveLength(1);
    expect(localStorage.getItem("bothq:draft:s-open")).toBe("still typing");
    localStorage.removeItem("bothq:draft:s-open");
    useChatStore.getState().clear("s-open");
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

// ===========================================================================
// The OS-notification wire, at its call site (EYES b96ab2cd)
//
// Deleting `useOsNotifications()` from Providers left 528 tests green — the
// definition, the import and the call were the only three hits in the tree.
// These tests render <Providers> and fire the real EVENTS, so they fail if
// the import, the call, the subscription, the queue, the flush or the send is
// cut. planFlush's policy has its own suite; this is only the wire.
// ===========================================================================

import { sendNotification } from "@tauri-apps/plugin-notification";

const mockSend = vi.mocked(sendNotification);

describe("Providers — the OS-notification wire", () => {
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
    mockInvoke.mockResolvedValue([]);
    mockSend.mockClear();
    localStorage.removeItem("bot-hq:os-notifications");
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("a pending_choice event while unfocused reaches sendNotification", async () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
    renderProviders();
    emit("session:pending_choice", {
      choice_id: "c1",
      session_id: "s1",
      agent: "hands",
      question: "merge or park?",
      options: ["merge", "park"],
    });
    await vi.advanceTimersByTimeAsync(2_000);
    expect(mockSend).toHaveBeenCalledTimes(1);
    expect(mockSend.mock.calls[0][0]).toMatchObject({
      body: "merge or park?",
    });
  });

  it("an awaiting_user event while unfocused reaches sendNotification", async () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
    renderProviders();
    emit("session:awaiting_user", {
      session_id: "s1",
      agent: "hands",
      reason: "waiting on the deploy window",
    });
    await vi.advanceTimersByTimeAsync(2_000);
    expect(mockSend).toHaveBeenCalledTimes(1);
    expect(mockSend.mock.calls[0][0]).toMatchObject({
      title: expect.stringContaining("waiting"),
    });
  });

  it("the Off toggle really silences escalation", async () => {
    const { setOsNotificationsEnabled } = await import("./lib/osNotifications");
    setOsNotificationsEnabled(false);
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
    renderProviders();
    emit("session:pending_choice", {
      choice_id: "c1",
      session_id: "s1",
      agent: "hands",
      question: "should not toast",
      options: ["ok"],
    });
    await vi.advanceTimersByTimeAsync(2_000);
    expect(mockSend).not.toHaveBeenCalled();
  });

  it("returning to the app during the burst window drops the queue", async () => {
    const focus = vi.spyOn(document, "hasFocus").mockReturnValue(false);
    renderProviders();
    emit("session:pending_choice", {
      choice_id: "c1",
      session_id: "s1",
      agent: "hands",
      question: "still there?",
      options: ["yes"],
    });
    focus.mockReturnValue(true); // user came back before the flush
    await vi.advanceTimersByTimeAsync(2_000);
    expect(mockSend).not.toHaveBeenCalled();
  });
});
