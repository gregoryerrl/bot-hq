// @vitest-environment node
/**
 * **One button** (the 2026-10-19 visual pass, S2g). S2b moved the 72 raw
 * buttons that drew their own outline or primary fill onto `ui/Button`; this
 * sweep keeps a new one from being hand-rolled. A raw `<button>` is still
 * right for a row, a tab, a switch or a segmented toggle — those draw no
 * button box and the detector passes them.
 *
 * Swept: every `.tsx` under `src/` except tests and `components/ui/` (the kit
 * itself). `import.meta.glob` for `typeScale.test.ts`'s reason.
 */
import { describe, expect, it } from "vitest";
import { findHandRolledButtons } from "./buttonLook";

const SOURCES = import.meta.glob("../**/*.tsx", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const swept = () =>
  Object.keys(SOURCES).filter((p) => !/\.test\.tsx$/.test(p) && !p.includes("/components/ui/"));

/**
 * Raw buttons that keep a box of their own: the file, a class that identifies
 * the button, and why it is not the kit's `<Button>`. The last test fails the
 * moment an entry stops matching.
 */
const ALLOWLIST: { file: string; marker: string; reason: string }[] = [
  {
    file: "app/ClaudeConfig.tsx",
    marker: "rounded-full",
    reason: "an on/off switch (role=switch): a track and a knob, not a button face",
  },
  {
    file: "components/DocumentPane.tsx",
    marker: "border-dashed",
    reason: "the new-document tab: a dashed outline the size of the tabs beside it, the row's 'add a tab' affordance",
  },
  {
    file: "components/PendingTray.tsx",
    marker: "relative",
    reason: "the top bar's notification bell: its count badge sits over the corner, and it lights in the primary tint when sessions wait",
  },
  {
    file: "components/ToolBurst.tsx",
    marker: "w-full",
    reason: "a tool burst's folded line: a full-width row that opens the burst, not an action button",
  },
];

const allowed = (file: string, classes: string) =>
  ALLOWLIST.some((a) => file.endsWith(`/${a.file}`) && classes.split(" ").includes(a.marker));

describe("one button", () => {
  it("no component hand-rolls a button look outside the kit", () => {
    const offenders = swept().flatMap((file) =>
      findHandRolledButtons(SOURCES[file])
        .filter((h) => !allowed(file, h.classes))
        .map((h) => `${file.replace("../", "")}:${h.line}: a raw <button> drawn ${h.look} — use ui/Button (${h.classes})`),
    );
    expect(offenders).toEqual([]);
  });

  it("reads the frontend source tree", () => {
    expect(swept().length).toBeGreaterThan(60);
  });

  it("flags a boxed or filled raw button and passes rows, tabs and the kit", () => {
    const looks = (src: string) => findHandRolledButtons(src).map((h) => h.look);
    expect(looks(`<button type="button" onClick={() => go()} className="rounded border border-outline-variant px-2 py-1">Copy</button>`)).toEqual(["boxed"]);
    expect(looks(`<button className={cn("rounded border px-3", on ? "bg-primary" : "")}>On</button>`)).toEqual(["filled"]);
    for (const ok of [
      `<Button variant="secondary">Copy</Button>`,
      `<button type="button" role="tab" className="border-b-2 px-3 py-2 font-code-sm">Roles</button>`,
      `<button type="button" className="flex w-full items-center gap-2 px-2 py-1 text-left hover:bg-surface-container-high">row</button>`,
      `<button type="button" onClick={() => a > b} className="underline">dismiss</button>`,
    ]) {
      expect(looks(ok), ok).toEqual([]);
    }
  });

  it("keeps every allowlist entry earning its place", () => {
    for (const entry of ALLOWLIST) {
      const key = Object.keys(SOURCES).find((k) => k.endsWith(`/${entry.file}`));
      expect(key, `${entry.file} must be in the swept set`).toBeDefined();
      expect(entry.reason.trim().length, `${entry.file}: say why`).toBeGreaterThan(20);
      expect(
        findHandRolledButtons(SOURCES[key as string]).some((h) => h.classes.split(" ").includes(entry.marker)),
        `${entry.file} no longer needs its entry — delete it`,
      ).toBe(true);
    }
  });
});
