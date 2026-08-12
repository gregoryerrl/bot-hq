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
    native: false,
    context_window: null,
  },
];

function role(over: Partial<RoleView> = {}): RoleView {
  return {
    id: 1,
    slug: "hands",
    display_name: "HANDS",
    description_prompt: "You are the hands.",
    capabilities: ["read_channel", "post_channel", "run_bash"],
    participation_mode: "active",
    default_model_id: null,
    builtin: true,
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
  builtin: true,
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

const prose = () => screen.getByRole("textbox", { name: /role instruction/i });
const nameField = () => screen.getByRole("textbox", { name: /display name/i });
const modeSelect = () =>
  screen.getByRole("combobox", { name: /participation mode/i });

describe("RolesPanel", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("lists every role and opens the first one's instruction", async () => {
    mockBackend();
    renderPanel();

    // Both roles reach the rail, each with its slug and the built-in mark.
    expect(await screen.findByText("HANDS")).toBeInTheDocument();
    expect(screen.getByText("EYES")).toBeInTheDocument();
    expect(within(screen.getByRole("list")).getByText("hands")).toBeInTheDocument();
    expect(screen.getAllByText("built-in")).toHaveLength(2);
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
      builtin: false,
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
          builtin: false,
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
