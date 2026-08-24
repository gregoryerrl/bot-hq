import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RolesPanel } from "./RolesPanel";
import { invoke } from "@tauri-apps/api/core";
import type { CapabilityView, ModelView, RoleView } from "../lib/bindings";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

const CAPS: CapabilityView[] = [
  {
    slug: "read_channel",
    label: "Read the channel",
    description: "See the session's messages.",
    group: "Channel",
    requires: [],
  },
  {
    slug: "post_channel",
    label: "Post to the channel",
    description: "Speak in the session channel.",
    group: "Channel",
    requires: [],
  },
  {
    slug: "run_bash",
    label: "Run Bash",
    description: "Run shell commands.",
    group: "Execution",
    requires: [],
  },
  {
    slug: "gated_bash",
    label: "Route a gated command",
    description: "Send a gated command to the user for approval.",
    group: "Execution",
    requires: ["run_bash"],
  },
];

const MODELS: ModelView[] = [
  {
    id: "m1",
    display_name: "Opus",
    provider: "anthropic",
    model_name: "claude-opus",
    base_url: null,
    auth_token: null,
    created_at: "",
    updated_at: "",
    context_window: null,
  },
];

/**
 * `builtin: false` is not a choice — it is the only value the database can
 * hold. Migration 0048 set the column to 0 on every row, `create_role`
 * hardcodes 0 and `update_role` never writes it. The fixture used to default it
 * to `true`, which is why the panel could branch on it wrongly for every real
 * role while these tests stayed green: a fixture asserting a state the schema
 * can no longer produce tests nothing.
 *
 * slug, so it defaults to `true` here to match this fixture's `hands`.
 */
function role(over: Partial<RoleView> = {}): RoleView {
  return {
    id: 1,
    slug: "hands",
    display_name: "HANDS",
    description_prompt: "You are the hands.",
    capabilities: ["read_channel", "post_channel", "run_bash"],
    participation_mode: "active",
    default_model_id: null,
    builtin: false,
    archived: false,
    ...over,
  };
}

const EYES = role({
  id: 2,
  slug: "eyes",
  display_name: "EYES",
  description_prompt: "You are the eyes.",
  capabilities: ["read_channel", "post_channel"],
});

/** Wires the three reads the panel makes; `roles` overrides the list. */
function mockBackend(roles: RoleView[] = [role(), EYES]) {
  mockInvoke.mockImplementation(async (cmd: string) => {
    if (cmd === "list_roles") return roles;
    if (cmd === "list_models") return MODELS;
    if (cmd === "list_capabilities") return CAPS;
    if (cmd === "create_role") return role({ id: 9, slug: "code-reviewer" });
    if (cmd === "update_role") return roles[0];
    if (cmd === "archive_role") return null;
    if (cmd === "get_claude_overrides")
      return { per_role: { hands: { effort: "max" } }, _all: { effort: "high" } };
    if (cmd === "claude_config_read") return { core_knobs: [] };
    if (cmd === "set_claude_overrides") return null;
    return undefined;
  });
}

function renderPanel() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <RolesPanel />
    </QueryClientProvider>,
  );
}

/** Just enough of `process` to watch for a promise nobody caught. */
type RejectionHub = {
  on(event: "unhandledRejection", handler: (reason: unknown) => void): void;
  off(event: "unhandledRejection", handler: (reason: unknown) => void): void;
};

const prose = () => screen.getByRole("textbox", { name: /role instruction/i });
const nameField = () => screen.getByRole("textbox", { name: /display name/i });
const modeSelect = () =>
  screen.getByRole("combobox", { name: /participation mode/i });

describe("RolesPanel", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("lists every role and opens the first one's instruction", async () => {
    mockBackend();
    renderPanel();

    // Both roles reach the rail, each with its slug.
    expect(await screen.findByText("HANDS")).toBeInTheDocument();
    expect(screen.getByText("EYES")).toBeInTheDocument();
    expect(within(screen.getByRole("list")).getByText("hands")).toBeInTheDocument();
    // No "built-in" chip. It branched on `builtin`, which 0048 made 0 for every
    // row, so it was dead markup claiming bot-hq ships these roles; the markup
    // is gone too. This asserts the rendered state, not the deletion — with the
    // flag false the chip could not appear either way. What keeps the flag
    // false is `no_role_is_flagged_builtin_after_0048` on the Rust side.
    expect(screen.queryByText("built-in")).toBeNull();
    // ...and the first is open in the detail pane, prose and all.
    expect(prose()).toHaveValue("You are the hands.");
    expect(mockInvoke).toHaveBeenCalledWith("list_roles", {
      includeArchived: false,
    });
  });

  it("re-lists with archived rows when the toggle is ticked", async () => {
    mockBackend();
    renderPanel();
    await screen.findByText("HANDS");

    fireEvent.click(screen.getByRole("checkbox", { name: /show archived/i }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("list_roles", {
        includeArchived: true,
      }),
    );
  });

  it("creates a role from the draft the form built", async () => {
    mockBackend();
    renderPanel();
    await screen.findByText("HANDS");

    fireEvent.click(screen.getByRole("button", { name: /\+ new role/i }));
    fireEvent.change(nameField(), { target: { value: "Code Reviewer" } });
    fireEvent.change(prose(), { target: { value: "Be terse." } });
    fireEvent.change(modeSelect(), { target: { value: "on_mention" } });
    fireEvent.change(screen.getByRole("combobox", { name: /default model/i }), {
      target: { value: "m1" },
    });
    fireEvent.click(screen.getByRole("checkbox", { name: /read the channel/i }));

    fireEvent.click(screen.getByRole("button", { name: /create role/i }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("create_role", {
        draft: {
          display_name: "Code Reviewer",
          // `null`, not a derived string: storage derives the slug on create.
          slug: null,
          description_prompt: "Be terse.",
          capabilities: ["read_channel"],
          participation_mode: "on_mention",
          default_model_id: "m1",
        },
      }),
    );
  });

  it("saves an edited instruction through update_role, keeping the slug", async () => {
    mockBackend();
    renderPanel();
    await screen.findByText("HANDS");

    fireEvent.change(prose(), { target: { value: "You are the HANDS, rewritten." } });
    fireEvent.click(screen.getByRole("button", { name: /save role/i }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("update_role", {
        id: 1,
        draft: expect.objectContaining({
          display_name: "HANDS",
          description_prompt: "You are the HANDS, rewritten.",
          // `slug: null` on update means LEAVE IT ALONE. Sending "hands" back
          // would be a rename, and `ensure_session_roster` resolves the seeded
          // roles by that literal slug.
          slug: null,
        }),
      }),
    );
  });

  it("shows a Validation refusal on the form instead of swallowing it", async () => {
    mockBackend();
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_roles") return [role()];
      if (cmd === "list_models") return MODELS;
      if (cmd === "list_capabilities") return CAPS;
      if (cmd === "update_role")
        throw { kind: "Validation", message: "`gated_bash` requires `run_bash`" };
      return undefined;
    });
    renderPanel();
    await screen.findByText("HANDS");

    fireEvent.change(prose(), { target: { value: "edited" } });
    fireEvent.click(screen.getByRole("button", { name: /save role/i }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("`gated_bash` requires `run_bash`");
    // And the capabilities block is marked, so the message points somewhere.
    expect(screen.getByRole("group", { name: /capabilities/i })).toHaveClass(
      "border-error",
    );
  });

  it("asks before archiving, and archives only once confirmed", async () => {
    mockBackend();
    renderPanel();
    await screen.findByText("HANDS");

    fireEvent.click(screen.getByRole("button", { name: /archive role/i }));

    // The click alone must not remove anything.
    expect(mockInvoke).not.toHaveBeenCalledWith("archive_role", expect.anything());
    const dialog = await screen.findByRole("dialog", {
      name: /archive this role\?/i,
    });
    // The copy has to say it is not a delete.
    expect(dialog).toHaveTextContent(/nothing is deleted/i);

    fireEvent.click(within(dialog).getByRole("button", { name: /^archive$/i }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("archive_role", {
        id: 1,
        archived: true,
      }),
    );
  });

  it("reports a failed Restore instead of leaving the role archived in silence", async () => {
    // Restore had no try/catch and rendered nothing: the mutation rejected
    // unhandled, the role stayed archived, and the screen was identical to the
    // click never having registered. Archive, one button away, wraps its call
    // and renders `archive.error` in its dialog — the asymmetry was the bug.
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_roles") return [role({ archived: true })];
      if (cmd === "list_models") return MODELS;
      if (cmd === "list_capabilities") return CAPS;
      if (cmd === "archive_role")
        throw { kind: "Internal", message: "database is locked" };
      return undefined;
    });
    renderPanel();
    await screen.findByText("HANDS");

    // Both halves of the defect are asserted, because they are separate: the
    // rejection escaping is what "unhandled" means, and the missing alert is
    // what "the user sees nothing" means. Removing either fix alone leaves one
    // of them live.
    const escaped: unknown[] = [];
    const onRejection = (reason: unknown) => escaped.push(reason);
    // Typed structurally rather than via `@types/node`: the frontend tsconfig
    // has no `node` types and adding them to see one event is a bad trade.
    // jsdom does not track unhandled rejections, so the host does.
    const hub = (globalThis as unknown as { process: RejectionHub }).process;
    hub.on("unhandledRejection", onRejection);
    try {
      fireEvent.click(screen.getByRole("button", { name: /restore role/i }));

      // The failure reaches the user, and says what went wrong.
      expect(await screen.findByText(/restore failed/i)).toHaveTextContent(
        "database is locked",
      );
      // Drain the microtask queue and one macrotask, which is when node
      // decides a rejection was never handled.
      await new Promise((r) => setTimeout(r, 0));
      expect(escaped).toEqual([]);
    } finally {
      hub.off("unhandledRejection", onRejection);
    }
    // And it is announced, not just drawn — this is the only signal that the
    // role is still archived.
    expect(screen.getByText(/restore failed/i)).toHaveAttribute("role", "alert");
    // The button is still there to try again; nothing pretended to succeed.
    expect(
      screen.getByRole("button", { name: /restore role/i }),
    ).toBeInTheDocument();
  });

  it("says an emptied instruction stays empty (Batch 4: the fallback is deleted)", async () => {
    // Clearing the box now means CLEARED: the compiled-prose fallback was
    // deleted with the neutral default role, so `description_prompt: null`
    // spawns the role with no layer-3 prose at all — briefed by the universal
    // rules and its capability grants. One arm — the built-in-prose flag is
    // deleted from the view entirely (Batch 6 D2).
    mockBackend([role()]);
    renderPanel();
    await screen.findByText("HANDS");

    // Nothing shouts while there is prose in the box.
    expect(screen.queryByText(/empty means empty/i)).toBeNull();

    fireEvent.change(prose(), { target: { value: "   \n  " } });

    const notice = await screen.findByText(/empty means empty/i);
    expect(notice).toHaveTextContent(/no instruction of its own/i);
    expect(notice).not.toHaveTextContent(/restore the default/i);
    expect(notice).not.toHaveTextContent(/built-in/i);

    // And the save really does send `null`, so the notice is describing what
    // happens rather than a second, separate rule.
    fireEvent.click(screen.getByRole("button", { name: /save role/i }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("update_role", {
        id: 1,
        draft: expect.objectContaining({ description_prompt: null }),
      }),
    );
  });

  it("offers exactly two participation modes, and both of them do something", async () => {
    mockBackend();
    renderPanel();
    await screen.findByText("HANDS");

    const options = within(modeSelect()).getAllByRole("option");
    expect(options.map((o) => o.getAttribute("value"))).toEqual([
      "active",
      "on_mention",
    ]);
    // rc3 D18: `observer` was the third. It was spawned, handed no turn and
    // delivered nothing — a process that read nothing, said nothing and billed
    // for existing.
    expect(modeSelect()).not.toHaveTextContent(/observer/i);
  });

  it("keeps a mode the picker no longer offers visible rather than rewriting it", async () => {
    // The picker omitting a value the row HOLDS is how editing the prose
    // silently changes the mode: the select falls back to its first option and
    // the save writes that back.
    //
    // `observer` is the fixture because it is the real case — rc3 D18 retired
    // it, and a database written before that can still hold one.
    mockBackend([role({ participation_mode: "observer" })]);
    renderPanel();
    await screen.findByText("HANDS");

    expect(modeSelect()).toHaveValue("observer");
    const stored = within(modeSelect()).getByRole("option", {
      name: /observer/i,
    });
    expect(stored).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: /save role/i }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("update_role", {
        id: 1,
        draft: expect.objectContaining({ participation_mode: "observer" }),
      }),
    );
  });

  it("drops capability slugs the backend no longer knows, and says so", async () => {
    // Migration 0044 seeded `hands` with `route_gated_command`, which
    // `Capability::parse` does not recognise — so submitting the stored list
    // back verbatim makes the HANDS role permanently unsaveable.
    mockBackend([
      role({ capabilities: ["read_channel", "route_gated_command"] }),
    ]);
    renderPanel();
    await screen.findByText("HANDS");

    expect(
      screen.getByText(/no longer recognises/i),
    ).toHaveTextContent("route_gated_command");

    fireEvent.click(screen.getByRole("button", { name: /save role/i }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("update_role", {
        id: 1,
        draft: expect.objectContaining({ capabilities: ["read_channel"] }),
      }),
    );
  });

  it("keeps a half-written instruction when the list refetches underneath", async () => {
    // The role instruction is the long-form field on this tab — someone can be
    // twenty minutes into one. `list_roles` is a React Query read that
    // refetches on its own (window focus, invalidation, the archived toggle),
    // and re-seeding the form from each new server row would throw that away
    // with no error and nothing to undo it.
    const RETIRED = role({
      id: 3,
      slug: "retired",
      display_name: "Retired",
      archived: true,
    });
    mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "list_roles")
        return (args as { includeArchived: boolean }).includeArchived
          ? [role(), EYES, RETIRED]
          : [role(), EYES];
      if (cmd === "list_models") return MODELS;
      if (cmd === "list_capabilities") return CAPS;
      return undefined;
    });
    renderPanel();
    await screen.findByText("HANDS");

    fireEvent.change(prose(), { target: { value: "half-written" } });
    fireEvent.click(screen.getByRole("checkbox", { name: /show archived/i }));

    expect(await screen.findByText("Retired")).toBeInTheDocument();
    expect(prose()).toHaveValue("half-written");
  });

  it("selects the role it just created, not the first in the list", async () => {
    // The new role is selected by id BEFORE the refetch that puts it in the
    // list, so a fall-back that fires on "no row for this id" lands the user on
    // someone else's role right after they pressed Create.
    const list: RoleView[] = [role(), EYES];
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_roles") return [...list];
      if (cmd === "list_models") return MODELS;
      if (cmd === "list_capabilities") return CAPS;
      if (cmd === "create_role") {
        const created = role({
          id: 9,
          slug: "code-reviewer",
          display_name: "Code Reviewer",
          description_prompt: "Be terse.",
          capabilities: [],
        });
        list.push(created);
        return created;
      }
      return undefined;
    });
    renderPanel();
    await screen.findByText("HANDS");

    fireEvent.click(screen.getByRole("button", { name: /\+ new role/i }));
    fireEvent.change(nameField(), { target: { value: "Code Reviewer" } });
    fireEvent.click(screen.getByRole("button", { name: /create role/i }));

    await waitFor(() =>
      expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent(
        "Code Reviewer",
      ),
    );
    expect(prose()).toHaveValue("Be terse.");
  });

  it("refuses to save while the capability list is missing", async () => {
    // Saving rewrites `capabilities` from the checklist, so an empty checklist
    // would write an empty grant list — stripping a role's permissions with no
    // error and nothing on screen that looked like a change.
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_roles") return [role()];
      if (cmd === "list_models") return MODELS;
      if (cmd === "list_capabilities") throw { kind: "Internal", message: "no" };
      return undefined;
    });
    renderPanel();
    await screen.findByText("HANDS");

    const save = screen.getByRole("button", { name: /save role/i });
    expect(save).toBeDisabled();
    fireEvent.click(save);
    expect(mockInvoke).not.toHaveBeenCalledWith("update_role", expect.anything());
  });

  it("renders the checklist from the backend, dependencies included", async () => {
    mockBackend();
    renderPanel();
    await screen.findByText("HANDS");

    // Grouped headings + one box per capability the command returned.
    expect(screen.getByText("Channel")).toBeInTheDocument();
    expect(screen.getByText("Execution")).toBeInTheDocument();
    expect(screen.getAllByRole("checkbox", { checked: true })).toHaveLength(3);

    // Ticking `gated_bash` without `run_bash` is refused by the backend, so the
    // form says so before the save rather than after it.
    fireEvent.click(
      screen.getByRole("checkbox", { name: /route a gated command/i }),
    );
    fireEvent.click(screen.getByRole("checkbox", { name: /run bash/i }));
    expect(await screen.findByText(/needs run_bash/i)).toBeInTheDocument();
  });
});

describe("default effort (no-inherit, 2026-08-25)", () => {
  it("shows the role's stored default with no Inherit option, and writes the decomposed pair", async () => {
    mockBackend();
    renderPanel();
    await screen.findByText("HANDS");

    // HANDS carries a stored per-role effort.
    const select = (await screen.findByLabelText(
      "Default effort",
    )) as HTMLSelectElement;
    // waitFor: the select renders (showing the floor) before the overrides
    // query lands the stored value.
    await waitFor(() => expect(select.value).toBe("max"));
    // No Inherit anywhere: concrete choices only.
    expect(
      Array.from(select.options).map((o) => o.textContent ?? ""),
    ).not.toContainEqual(expect.stringMatching(/inherit/i));

    // Changing writes the WHOLE store back with only this slug's slot moved —
    // a level pick carries the explicit ultracode:false half of the pair, and
    // `_all` (dead for effort, still real for its other keys) rides through.
    fireEvent.change(select, { target: { value: "low" } });
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "set_claude_overrides",
        expect.objectContaining({
          overrides: expect.objectContaining({
            per_role: expect.objectContaining({
              hands: expect.objectContaining({ effort: "low", ultracode: false }),
            }),
            _all: expect.objectContaining({ effort: "high" }),
          }),
        }),
      ),
    );
  });

  it("shows the medium floor for an unconfigured role, and gates ultracode on edit_files", async () => {
    mockBackend();
    renderPanel();
    // EYES has no per_role entry and no edit_files capability.
    fireEvent.click(await screen.findByText("EYES"));

    const select = (await screen.findByLabelText(
      "Default effort",
    )) as HTMLSelectElement;
    // Display mirrors the spawn floor — never `_all`'s high, never blank.
    await waitFor(() => expect(select.value).toBe("medium"));
    const ultracode = select.querySelector(
      'option[value="ultracode"]',
    ) as HTMLOptionElement;
    expect(ultracode).not.toBeNull();
    // A default this role cannot take is not offered: ultracode rides in on
    // `--settings`, which spawn injects only for a role holding edit_files.
    expect(ultracode.disabled).toBe(true);
  });

  it("stores an ultracode default as the xhigh+ultracode pair", async () => {
    // A HANDS that can edit files — the capability the ultracode option keys on.
    mockBackend([
      role({ capabilities: ["read_channel", "edit_files"] }),
      EYES,
    ]);
    renderPanel();
    await screen.findByText("HANDS");

    const select = (await screen.findByLabelText(
      "Default effort",
    )) as HTMLSelectElement;
    const ultracode = select.querySelector(
      'option[value="ultracode"]',
    ) as HTMLOptionElement;
    expect(ultracode.disabled).toBe(false);

    fireEvent.change(select, { target: { value: "ultracode" } });
    // xhigh is ultracode's implied level, stored explicitly so even a spawn
    // that skips `--settings` emits a truthful CLAUDE_CODE_EFFORT_LEVEL.
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "set_claude_overrides",
        expect.objectContaining({
          overrides: expect.objectContaining({
            per_role: expect.objectContaining({
              hands: expect.objectContaining({
                effort: "xhigh",
                ultracode: true,
              }),
            }),
          }),
        }),
      ),
    );
  });
});
