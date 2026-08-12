import { describe, it, expect } from "vitest";
import {
  authorLabel,
  capabilityGapWarning,
  participantLabelIndex,
  participantLabel,
  participantRuntime,
  slotKey,
  UNKNOWN_PARTICIPANT,
  type ParticipantView,
} from "./participants";

function p(over: Partial<ParticipantView> = {}): ParticipantView {
  return {
    id: 1,
    slug: "hands",
    role_display_name: "HANDS",
    model_display_name: "Claude Opus 5",
    turn_position: 0,
    participation_mode: "active",
    enabled: true,
    ...over,
  };
}

describe("participantLabel — the contract's display rule", () => {
  it("joins role and model with a middle dot", () => {
    expect(participantLabel(p())).toBe("HANDS · Claude Opus 5");
  });

  it("falls back to the model alone when the role is gone", () => {
    expect(participantLabel(p({ role_display_name: null }))).toBe(
      "Claude Opus 5",
    );
  });

  it("falls back to the role alone when no model is set", () => {
    expect(participantLabel(p({ model_display_name: null }))).toBe("HANDS");
  });

  it("falls back to the slug only when both are null", () => {
    // The one path that can put an internal key on screen, and only when there
    // is nothing else to say.
    expect(
      participantLabel(
        p({ role_display_name: null, model_display_name: null }),
      ),
    ).toBe("hands");
  });

  it("treats a blank display name as absent, not as a label", () => {
    // A whitespace-only name would otherwise render as an empty byline.
    expect(participantLabel(p({ role_display_name: "   " }))).toBe(
      "Claude Opus 5",
    );
  });
});

describe("authorLabel", () => {
  const labels = participantLabelIndex([
    p(),
    p({
      id: 2,
      slug: "eyes",
      role_display_name: "EYES",
      model_display_name: "R2",
      turn_position: 1,
    }),
  ]);

  it("resolves a participant slug through the roster", () => {
    expect(authorLabel("hands", labels)).toBe("HANDS · Claude Opus 5");
    expect(authorLabel("eyes", labels)).toBe("EYES · R2");
  });

  it("names the non-participant authors without inventing a role", () => {
    expect(authorLabel("user", labels)).toBe("You");
    expect(authorLabel("system", labels)).toBe("System");
  });

  it("lets the roster win over a reserved word", () => {
    // A role could legitimately be slugged `user`; the session's own roster is
    // the authority on who its participants are.
    const shadowed = participantLabelIndex([p({ slug: "user" })]);
    expect(authorLabel("user", shadowed)).toBe("HANDS · Claude Opus 5");
  });

  it("names an unresolvable author without printing its slug", () => {
    // rc3 D10 kept legacy rows renderable — "brian and rain's history can be
    // legacy data" — and every one of them carries `author = 'brian'` or
    // `'rain'`. Falling back to the slug put exactly the two names the decision
    // removed back on screen, so an author the roster cannot back is named by
    // what is actually known about it: nothing.
    expect(authorLabel("departed", labels)).toBe(UNKNOWN_PARTICIPANT);
    expect(authorLabel("brian", labels)).toBe(UNKNOWN_PARTICIPANT);
    expect(authorLabel("rain", labels)).toBe(UNKNOWN_PARTICIPANT);
    expect(UNKNOWN_PARTICIPANT).not.toMatch(/brian|rain/i);
    // Still attributed — an empty byline would be its own defect.
    expect(authorLabel("departed", labels)).not.toBe("");
    expect(authorLabel(null, labels)).toBe("");
  });

  it("returns a STRING for an author that collides with Object.prototype", () => {
    // A bare index answers `labels["toString"]` out of the prototype, and a
    // function is not null-ish, so `??` would not catch it — the byline would
    // render a function body. Every surface resolves its author through here.
    for (const author of ["toString", "constructor", "valueOf"]) {
      expect(authorLabel(author, labels)).toBe(UNKNOWN_PARTICIPANT);
    }
    // …and a real roster row under such a slug still wins.
    const odd = participantLabelIndex([p({ slug: "toString" })]);
    expect(authorLabel("toString", odd)).toBe("HANDS · Claude Opus 5");
  });
});

describe("the two runtime key spaces", () => {
  // Two of the backend's runtime payloads are a frozen fixed pair naming TURN
  // SLOTS (`SessionActivityEvent.brian_busy` / `SessionRuntime.brian_health`,
  // filled from `slugs.get(0)` / `participants.get(0)`), while the live
  // `session:agent_health` / `session:agent_context` events key by the
  // participant's slug. Both have to reach the same participant.
  const first = p({
    slug: "eyes",
    role_display_name: "EYES",
    turn_position: 0,
  });
  const second = p({
    id: 2,
    slug: "eyes-2",
    role_display_name: "EYES",
    model_display_name: "DeepSeek R2",
    turn_position: 1,
  });

  it("keeps a slot key from ever colliding with a slug", () => {
    // `slugify` (src/storage/participants.rs) emits `[a-z0-9-]` only, trimmed
    // of leading dashes and never empty — so a key outside that alphabet can
    // share one map with slugs and neither space can overwrite the other.
    expect(slotKey(0)).toBe("#slot0");
    expect(slotKey(1)).not.toBe(slotKey(0));
    for (const k of [slotKey(0), slotKey(1), slotKey(2)]) {
      expect(k).not.toMatch(/^[a-z0-9-]+$/);
    }
  });

  it("resolves a participant's runtime state through EITHER key space", () => {
    // The backfill seeds health by slot; the live event seeds it by slug. A
    // reader keyed to one space alone goes blank the moment the other produces.
    expect(participantRuntime({ [slotKey(0)]: "stalled" }, first)).toBe(
      "stalled",
    );
    expect(participantRuntime({ eyes: "dead" }, first)).toBe("dead");
    expect(participantRuntime({ [slotKey(1)]: "dead" }, second)).toBe("dead");
  });

  it("prefers the live slug over the snapshot slot", () => {
    // The slug comes from an event, the slot key from a mount-time snapshot.
    expect(
      participantRuntime({ eyes: "running", [slotKey(0)]: "dead" }, first),
    ).toBe("running");
  });

  it("reads nothing for a participant nothing has reported", () => {
    expect(participantRuntime({ "eyes-2": "dead" }, first)).toBeUndefined();
    expect(participantRuntime(undefined, first)).toBeUndefined();
    // A third participant has no slot on the wire at all (the pair reports 0
    // and 1) — it resolves by slug once the live events supply one.
    const third = p({ id: 3, slug: "hands-2", turn_position: 2 });
    expect(participantRuntime({ [slotKey(0)]: "dead" }, third)).toBeUndefined();
    expect(participantRuntime({ "hands-2": "running" }, third)).toBe("running");
  });

  it("indexes a label under BOTH keys, so one lookup serves either producer", () => {
    // This is what lets `authorLabel` be the single lookup for the chat byline
    // (slug-keyed) and the turn-status line (slot-keyed) at once.
    const labels = participantLabelIndex([first, second]);
    expect(labels["eyes"]).toBe("EYES · Claude Opus 5");
    expect(labels[slotKey(0)]).toBe("EYES · Claude Opus 5");
    expect(labels[slotKey(1)]).toBe("EYES · DeepSeek R2");
    expect(authorLabel(slotKey(0), labels)).toBe("EYES · Claude Opus 5");
  });
});

describe("capabilityGapWarning — rc3 D11", () => {
  const READ_ONLY = { capabilities: ["read_channel", "post_channel"] };
  const EDITOR = { capabilities: ["read_channel", "edit_files"] };

  it("warns when the union holds no edit_files", () => {
    expect(capabilityGapWarning([READ_ONLY])).toMatch(
      /no participant can edit files/i,
    );
  });

  it("warns the same way for two of the SAME role", () => {
    // Duplicate roles are not blocked and not special-cased — this is simply
    // one roster whose union is missing the box.
    expect(capabilityGapWarning([READ_ONLY, READ_ONLY])).toMatch(
      /no participant can edit files/i,
    );
  });

  it("is silent as soon as ONE participant holds edit_files", () => {
    expect(capabilityGapWarning([READ_ONLY, EDITOR])).toBeNull();
    expect(capabilityGapWarning([EDITOR])).toBeNull();
  });

  it("names what the SET cannot do, never who the roles are", () => {
    const msg = capabilityGapWarning([READ_ONLY])!;
    expect(msg).toMatch(/review, but nothing in it can act/i);
    expect(msg).not.toMatch(/reviewer|EYES|HANDS|brian|rain/i);
  });

  it("says nothing about an empty roster", () => {
    expect(capabilityGapWarning([])).toBeNull();
  });
});
