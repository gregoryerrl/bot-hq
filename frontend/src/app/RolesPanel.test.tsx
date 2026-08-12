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
 * `has_builtin_prose` is the real question and is derived in Rust from the
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
    has_builtin_prose: true,
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
    fireEvent.change(modeSelect(), { target: { value: "observer" } });
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
          participation_mode: "observer",
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

  it("says an emptied instruction falls back to the built-in text", async () => {
    // Clearing the box does NOT give the role a blank instruction: `submit`
    // sends `description_prompt: null` and the spawn path reads NULL as "use
    // the built-in", so emptying it reinstates the shipped prose. Correct
    // behaviour, but it was invisible, so a user clearing the box to silence a
    // role got the opposite.
    //
    // `builtin` is deliberately NOT set here. It is false on every real row,
    // and this arm must still be reached — that is the whole regression: the
    // panel branched on `builtin` and sent HANDS down the "no instruction" arm.
    mockBackend([role({ has_builtin_prose: true })]);
    renderPanel();
    await screen.findByText("HANDS");

    // Nothing shouts while there is prose in the box.
    expect(screen.queryByText(/empty is not a blank instruction/i)).toBeNull();

    fireEvent.change(prose(), { target: { value: "   \n  " } });

    const notice = await screen.findByText(/empty is not a blank instruction/i);
    // `’` in the source, so the apostrophe on screen is U+2019, not U+0027.
    expect(notice).toHaveTextContent(/falls back to bot-hq[’']s built-in text/i);
    // A role with prose behind it is the "restore defaults" case, and the copy
    // says so rather than warning about an instruction-less spawn.
    expect(notice).toHaveTextContent(/restore the default/i);
    expect(notice).not.toHaveTextContent(/no instruction of its own/i);

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

  it("does not promise a built-in for a role bot-hq carries no prose for", async () => {
    // `role_for` only knows the seeded agent slugs; anything else falls back to
    // an empty string and the section is skipped. Telling the author of a new
    // role that clearing the box "restores the default" would be a promise with
    // nothing behind it. The backend decides this via `has_builtin_prose`.
    mockBackend([
      role({
        id: 5,
        slug: "auditor",
        display_name: "Auditor",
        has_builtin_prose: false,
      }),
    ]);
    renderPanel();
    await screen.findByText("Auditor");

    fireEvent.change(prose(), { target: { value: "" } });

    const notice = await screen.findByText(/empty is not a blank instruction/i);
    expect(notice).toHaveTextContent(/no instruction of its own/i);
    expect(notice).not.toHaveTextContent(/restore the default/i);
  });

  it("never offers on_demand as a participation mode", async () => {
    mockBackend();
    renderPanel();
    await screen.findByText("HANDS");

    const options = within(modeSelect()).getAllByRole("option");
    expect(options.map((o) => o.getAttribute("value"))).toEqual([
      "active",
      "observer",
    ]);
    // rc3 D1: mention-wake is not built, so an on_demand role would be
    // enabled, rostered and never given a turn.
    expect(modeSelect()).not.toHaveTextContent("on_demand");
  });

  it("keeps a stored on_demand mode visible rather than rewriting it", async () => {
    // The picker omitting a value the row HOLDS is how editing the prose
    // silently changes the mode: the select falls back to its first option and
    // the save writes that back.
    mockBackend([role({ participation_mode: "on_demand" })]);
    renderPanel();
    await screen.findByText("HANDS");

    expect(modeSelect()).toHaveValue("on_demand");
    const stored = within(modeSelect()).getByRole("option", {
      name: /on_demand/i,
    });
    expect(stored).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: /save role/i }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("update_role", {
        id: 1,
        draft: expect.objectContaining({ participation_mode: "on_demand" }),
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
      has_builtin_prose: false,
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
          has_builtin_prose: false,
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
      expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent(
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
