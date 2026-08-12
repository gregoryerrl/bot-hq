import { describe, it, expect, vi } from "vitest";
import { seedRuntimeStores, type SessionRuntime } from "./runtime";
import {
  participantRuntime,
  slotKey,
  type ParticipantView,
} from "../lib/participants";

describe("seedRuntimeStores", () => {
  it("seeds activity for every row and health for non-null agents", () => {
    const setActivity = vi.fn();
    const setHealth = vi.fn();
    const setRouterHealth = vi.fn();
    const rows: SessionRuntime[] = [
      {
        session_id: "s1",
        activity: "busy",
        brian_busy: true,
        rain_busy: false,
        brian_health: "running",
        rain_health: "retrying",
        router_alive: false,
        attention: "idle_unflagged",
        working: "gates running",
      },
      {
        session_id: "s2",
        activity: "awaiting_user",
        brian_busy: false,
        rain_busy: false,
        brian_health: "dead",
        rain_health: null,
        router_alive: null,
        attention: null,
        working: null,
      },
    ];

    seedRuntimeStores(rows, setActivity, setHealth, setRouterHealth);

    // The `brian_*` / `rain_*` field names are frozen wire naming TURN SLOTS 0
    // and 1 — `src/tauri_cmd/sessions.rs` fills them from
    // `handle.participants.get(0)` / `.get(1)`. Unpacking them under the
    // LITERAL slugs `"brian"` / `"rain"` is what blanked every health dot after
    // a restart: no rc3 roster has those slugs, so the session header's
    // per-participant lookup missed every entry this seeds.
    expect(setActivity).toHaveBeenCalledWith("s1", "busy", {
      [slotKey(0)]: true,
      [slotKey(1)]: false,
    });
    expect(setActivity).toHaveBeenCalledWith("s2", "awaiting_user", {
      [slotKey(0)]: false,
      [slotKey(1)]: false,
    });
    expect(setHealth).toHaveBeenCalledWith("s1", slotKey(0), "running");
    expect(setHealth).toHaveBeenCalledWith("s1", slotKey(1), "retrying");
    expect(setHealth).toHaveBeenCalledWith("s2", slotKey(0), "dead");
    // No agent name reaches the store as a key.
    expect(setHealth).not.toHaveBeenCalledWith("s1", "brian", expect.anything());
    expect(setHealth).not.toHaveBeenCalledWith("s1", "rain", expect.anything());
    // s2.rain_health is null → no setHealth call for slot 1.
    expect(setHealth).not.toHaveBeenCalledWith(
      "s2",
      slotKey(1),
      expect.anything(),
    );
    expect(setHealth).toHaveBeenCalledTimes(3);
    // s1.router_alive is false → seeded; s2.router_alive is null → skipped.
    expect(setRouterHealth).toHaveBeenCalledWith("s1", false);
    expect(setRouterHealth).toHaveBeenCalledTimes(1);
  });

  it("seeds under keys the session header can actually resolve", () => {
    // The producer and the consumer are pinned separately everywhere else, so
    // this asserts the WIRE: what `seedRuntimeStores` writes is what
    // `participantRuntime` reads back for the participant occupying that slot.
    // A rekey on either side that forgets the other fails here.
    const roster: ParticipantView[] = [
      {
        id: 1,
        slug: "eyes",
        role_display_name: "EYES",
        model_display_name: "Claude Opus 5",
        turn_position: 0,
        participation_mode: "active",
        enabled: true,
      },
      {
        id: 2,
        slug: "eyes-2",
        role_display_name: "EYES",
        model_display_name: "DeepSeek R2",
        turn_position: 1,
        participation_mode: "active",
        enabled: true,
      },
    ];
    const health: Record<string, string | undefined> = {};
    const busy: Record<string, Record<string, boolean>> = {};
    seedRuntimeStores(
      [
        {
          session_id: "s1",
          activity: "busy",
          brian_busy: true,
          rain_busy: false,
          brian_health: "stalled",
          rain_health: "dead",
          router_alive: null,
          attention: null,
          working: null,
        },
      ],
      (id, _a, b) => {
        busy[id] = b as Record<string, boolean>;
      },
      (_id, agent, h) => {
        health[agent] = h;
      },
      () => {},
    );

    expect(participantRuntime(health, roster[0])).toBe("stalled");
    expect(participantRuntime(health, roster[1])).toBe("dead");
    expect(participantRuntime(busy["s1"], roster[0])).toBe(true);
    expect(participantRuntime(busy["s1"], roster[1])).toBe(false);
  });

  it("is a no-op for an empty snapshot", () => {
    const setActivity = vi.fn();
    const setHealth = vi.fn();
    const setRouterHealth = vi.fn();
    seedRuntimeStores([], setActivity, setHealth, setRouterHealth);
    expect(setActivity).not.toHaveBeenCalled();
    expect(setHealth).not.toHaveBeenCalled();
    expect(setRouterHealth).not.toHaveBeenCalled();
  });
});
