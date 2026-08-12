import { describe, it, expect } from "vitest";
import {
  authorLabel,
  capabilityGapWarning,
  labelsBySlug,
  participantLabel,
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
  const labels = labelsBySlug([
    p(),
    p({ id: 2, slug: "eyes", role_display_name: "EYES", model_display_name: "R2" }),
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
    const shadowed = labelsBySlug([p({ slug: "user" })]);
    expect(authorLabel("user", shadowed)).toBe("HANDS · Claude Opus 5");
  });

  it("keeps an unknown author attributable rather than dropping it", () => {
    expect(authorLabel("departed", labels)).toBe("departed");
    expect(authorLabel(null, labels)).toBe("");
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
