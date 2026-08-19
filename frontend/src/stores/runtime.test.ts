import { describe, it, expect, vi } from "vitest";
import { busyBySlot, seedRuntimeStores, type SessionRuntime } from "./runtime";
import {
  authorLabel,
  participantLabelIndex,
  participantRuntime,
  slotKey,
  type ParticipantView,
} from "../lib/participants";

/** The roster both key spaces have to reach, in turn order. */
const ROSTER: ParticipantView[] = [
  {
    id: 1,
    slug: "eyes",
    role_display_name: "EYES",
    model_display_name: "Claude Opus 5",
    turn_position: 0,
    participation_mode: "active",
    color: null,
    label: null,
    effort: null,
    ultracode: null,
    effort_at_spawn: null,
    ultracode_at_spawn: null,
    spawn_knobs_recorded: false,
    enabled: true,
  },
  {
    id: 2,
    slug: "eyes-2",
    role_display_name: "EYES",
    model_display_name: "DeepSeek R2",
    turn_position: 1,
    participation_mode: "active",
    color: null,
    label: null,
    effort: null,
    ultracode: null,
    effort_at_spawn: null,
    ultracode_at_spawn: null,
    spawn_knobs_recorded: false,
    enabled: true,
  },
];

describe("busyBySlot — the frozen pair, unpacked once", () => {
  // `Providers.tsx` calls this on every `session:activity` event and
  // `seedRuntimeStores` calls it on the mount snapshot. It is the ONLY place
  // the pair is unpacked, so the live event and the backfill cannot key the
  // same session two different ways.
  it("keys the pair by turn slot, never by an agent name", () => {
    const busy = busyBySlot({ slot0_busy: true, slot1_busy: false });
    expect(busy).toEqual({ "#slot0": true, "#slot1": false });
    expect(Object.keys(busy)).not.toContain("brian");
    expect(Object.keys(busy)).not.toContain("rain");
  });

  it("names every busy slot, not only the frozen pair (round 12)", () => {
    // A roster of three: the third participant's busy edge is invisible to
    // the pair and used to be invisible to the UI.
    const busy = busyBySlot({ slot0_busy: false, slot1_busy: false, busy_slots: [2] });
    expect(busy).toEqual({ "#slot0": false, "#slot1": false, "#slot2": true });
    // An older payload without the list still unpacks the pair.
    expect(busyBySlot({ slot0_busy: true, slot1_busy: false })).toEqual({
      "#slot0": true,
      "#slot1": false,
    });
    // …and the third slot resolves to its participant on the status line, the
    // same way slots 0 and 1 do — three working participants, three names.
    const third: ParticipantView = {
      ...ROSTER[1]!,
      id: 3,
      slug: "hands-2",
      role_display_name: "HANDS",
      model_display_name: "Claude Fable 5",
      turn_position: 2,
    };
    const labels = participantLabelIndex([...ROSTER, third]);
    const all = busyBySlot({ slot0_busy: true, slot1_busy: true, busy_slots: [0, 1, 2] });
    const working = Object.keys(all).filter((k) => all[k]);
    expect(working.map((k) => authorLabel(k, labels))).toEqual([
      "EYES · Claude Opus 5",
      "EYES-2 · DeepSeek R2",
      "HANDS-2 · Claude Fable 5",
    ]);
  });

  it("lands under keys the turn-status line resolves to a participant", () => {
    // The wire, end to end: the event payload goes in, and the label the status
    // line prints comes out — via the same index the chat byline uses. Cutting
    // either half (the unpack keys, or the label index's slot entries) fails.
    const busy = busyBySlot({ slot0_busy: false, slot1_busy: true });
    const labels = participantLabelIndex(ROSTER);
    const working = Object.keys(busy).filter((k) => busy[k]);
    expect(working.map((k) => authorLabel(k, labels))).toEqual([
      "EYES-2 · DeepSeek R2",
    ]);
    // …and it is reachable as runtime state for that same participant row.
    expect(participantRuntime(busy, ROSTER, ROSTER[1])).toBe(true);
    expect(participantRuntime(busy, ROSTER, ROSTER[0])).toBe(false);
  });
});

describe("seedRuntimeStores", () => {
  it("seeds activity for every row and health for non-null agents", () => {
    const setActivity = vi.fn();
    const setHealth = vi.fn();
    const rows: SessionRuntime[] = [
      {
        session_id: "s1",
        activity: "busy",
        slot0_busy: true,
        slot1_busy: false,
        busy_slots: [0],
        slot0_health: "running",
        slot1_health: "retrying",
        attention: "idle_unflagged",
      },
      {
        session_id: "s2",
        activity: "awaiting_user",
        slot0_busy: false,
        slot1_busy: false,
        busy_slots: [],
        slot0_health: "dead",
        slot1_health: null,
        attention: null,
      },
    ];

    seedRuntimeStores(rows, setActivity, setHealth);

    // The `slot0_*` / `slot1_*` field names name TURN SLOTS 0 and 1 —
    // `src/tauri_cmd/sessions.rs` fills them from
    // `handle.participants.get(0)` / `.get(1)`. They were `brian_*` / `rain_*`
    // until the D10 hard retirement, and unpacking them under the LITERAL slugs
    // `"brian"` / `"rain"` is what blanked every health dot after a restart: no
    // rc3 roster has those slugs, so the session header's per-participant
    // lookup missed every entry this seeds.
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
    // s2.slot1_health is null → no setHealth call for slot 1.
    expect(setHealth).not.toHaveBeenCalledWith(
      "s2",
      slotKey(1),
      expect.anything(),
    );
    expect(setHealth).toHaveBeenCalledTimes(3);
  });

  it("seeds under keys the session header can actually resolve", () => {
    // The producer and the consumer are pinned separately everywhere else, so
    // this asserts the WIRE: what `seedRuntimeStores` writes is what
    // `participantRuntime` reads back for the participant occupying that slot.
    // A rekey on either side that forgets the other fails here.
    const health: Record<string, string | undefined> = {};
    const busy: Record<string, Record<string, boolean>> = {};
    seedRuntimeStores(
      [
        {
          session_id: "s1",
          activity: "busy",
          slot0_busy: true,
          slot1_busy: false,
          busy_slots: [0],
          slot0_health: "stalled",
          slot1_health: "dead",
          attention: null,
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

    expect(participantRuntime(health, ROSTER, ROSTER[0])).toBe("stalled");
    expect(participantRuntime(health, ROSTER, ROSTER[1])).toBe("dead");
    expect(participantRuntime(busy["s1"], ROSTER, ROSTER[0])).toBe(true);
    expect(participantRuntime(busy["s1"], ROSTER, ROSTER[1])).toBe(false);
  });

  it("is a no-op for an empty snapshot", () => {
    const setActivity = vi.fn();
    const setHealth = vi.fn();
    seedRuntimeStores([], setActivity, setHealth);
    expect(setActivity).not.toHaveBeenCalled();
    expect(setHealth).not.toHaveBeenCalled();
  });
});
