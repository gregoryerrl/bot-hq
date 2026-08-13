import { PARTICIPANT_COLORS } from "../components/authorColor";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { Dashboard, MAX_PARTICIPANTS } from "./Dashboard";
import { invoke } from "@tauri-apps/api/core";
import type { ClaudeOverrides, ModelView, RoleView } from "../lib/bindings";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
// The dashboard subscribes to `agent:messages:batch` for Quickview liveness.
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));
const mockInvoke = vi.mocked(invoke);

const MODELS: ModelView[] = [
  {
    id: "m-opus",
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

function role(over: Partial<RoleView> = {}): RoleView {
  return {
    id: 1,
    slug: "hands",
    display_name: "HANDS",
    description_prompt: null,
    capabilities: ["read_channel", "edit_files"],
    participation_mode: "active",
    default_model_id: null,
    builtin: false,
    // `hands` is one of the two slugs `builtin_prose_for_role` answers for, so
    // the honest default for this fixture's slug is true.
    has_builtin_prose: true,
    archived: false,
    ...over,
  };
}

const EYES = role({ id: 2, slug: "eyes", display_name: "EYES" });

/**
 * Wires every read the dashboard makes; `roles` overrides the role list and
 * `claude` the effort-inheritance sources (override store + settings.json knob).
 */
function mockBackend(
  roles: RoleView[] = [role(), EYES],
  claude: { overrides?: ClaudeOverrides; knob?: string | null } = {},
) {
  mockInvoke.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "list_sessions":
        return [];
      case "list_pending_tray":
        return [];
      case "list_projects":
        return [];
      case "list_models":
        return MODELS;
      case "list_roles":
        return roles;
      case "get_app_setting":
        return null;
      case "get_claude_overrides":
        return claude.overrides ?? {};
      case "claude_config_read":
        return {
          core_knobs:
            claude.knob === undefined
              ? []
              : [
                  {
                    key: "env.CLAUDE_CODE_EFFORT_LEVEL",
                    label: "Effort level",
                    value: claude.knob,
                    source: "~/.claude/settings.json",
                    inheritance: {
                      inherited_by: [],
                      skipped_by: [],
                      note: "",
                      overridable: true,
                    },
                  },
                ],
        };
      case "create_session":
        return { id: "s-new" };
      default:
        return undefined;
    }
  });
}

function renderDashboard() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <Dashboard />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

/** Open the New session dialog and wait for the role list to arrive. */
async function openDialog() {
  renderDashboard();
  fireEvent.click(screen.getByRole("button", { name: /new session/i }));
  await screen.findByRole("dialog", { name: /new session/i });
}

const roleSelect = (n: number) =>
  screen.getByRole("combobox", { name: `Participant ${n} role` });
const modelSelect = (n: number) =>
  screen.getByRole("combobox", { name: `Participant ${n} model` });
const effortSelect = (n: number) =>
  screen.getByRole("combobox", { name: `Participant ${n} effort` });
const ultracodeBox = (n: number) =>
  screen.getByRole("checkbox", { name: `Participant ${n} ultracode` });
const createButton = () =>
  screen.getByRole("button", { name: /create session/i });

/** The row's "Inherit (…)" option text — what its effort resolves to today. */
const inheritOption = (n: number) =>
  effortSelect(n).querySelector('option[value=""]')!.textContent;

/** The options object the dialog actually sent. */
function sentOptions() {
  const call = mockInvoke.mock.calls.find((c) => c[0] === "create_session");
  return (call?.[1] as { options: Record<string, unknown> }).options;
}

describe("New session dialog — participants", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("opens with one participant and no role chosen", async () => {
    mockBackend();
    await openDialog();

    // Design §1: how many agents, default 1.
    expect(roleSelect(1)).toBeInTheDocument();
    expect(
      screen.queryByRole("combobox", { name: "Participant 2 role" }),
    ).not.toBeInTheDocument();
    // Nothing is pre-picked: a guessed role is a session running with an agent
    // the user did not choose.
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));
    fireEvent.change(screen.getByPlaceholderText(/refactor auth flow/i), {
      target: { value: "a task" },
    });
    expect(createButton()).toBeDisabled();
  });

  it("sends the picked roles in turn order, with the model override", async () => {
    mockBackend();
    await openDialog();
    fireEvent.change(screen.getByPlaceholderText(/refactor auth flow/i), {
      target: { value: "a task" },
    });
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));

    fireEvent.change(roleSelect(1), { target: { value: "1" } });
    fireEvent.click(screen.getByRole("button", { name: /add participant/i }));
    fireEvent.change(roleSelect(2), { target: { value: "2" } });
    // The model picker overrides the role's default for THIS participant (D8).
    fireEvent.change(modelSelect(2), { target: { value: "m-opus" } });

    expect(createButton()).toBeEnabled();
    fireEvent.click(createButton());

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "create_session",
        expect.objectContaining({
          options: expect.objectContaining({
            participants: [
              { roleId: 1, modelId: null, effort: null, ultracode: null, color: null },
              { roleId: 2, modelId: "m-opus", effort: null, ultracode: null, color: null },
            ],
          }),
        }),
      ),
    );
    // The roster is the single source: the solo flag and both model columns are
    // derived from it backend-side, so the dialog must not also assert them.
    const args = mockInvoke.mock.calls.find((c) => c[0] === "create_session")?.[1] as {
      rainEnabled: boolean | null;
      brianModelId: string | null;
      rainModelId: string | null;
    };
    expect(args.rainEnabled).toBeNull();
    expect(args.brianModelId).toBeNull();
    expect(args.rainModelId).toBeNull();
  });

  it("stops at the participant cap, wherever the cap is set", async () => {
    mockBackend();
    await openDialog();
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));

    // Driven off MAX_PARTICIPANTS rather than a hardcoded 2, so raising the cap
    // is a one-line change and not a test rewrite. The cap is a PRODUCT choice,
    // not a runtime limit: the backend accepts MAX_SESSION_PARTICIPANTS = 8 and
    // spawn iterates the roster. It is low because each participant is its own
    // claude-code subprocess with its own context window and its own bill.
    // (The reason that used to sit here — "spawn starts two literally-named
    // agents" — stopped being true when D10 made spawn roster-driven and the
    // bilateral router was deleted. A stale REASON on a live assertion is how
    // someone later concludes the cap cannot be raised.)
    const addButton = () =>
      screen.queryByRole("button", { name: /add participant/i });

    for (let n = 2; n <= MAX_PARTICIPANTS; n++) {
      fireEvent.click(addButton()!);
      expect(roleSelect(n)).toBeInTheDocument();
    }
    expect(addButton()).not.toBeInTheDocument();

    fireEvent.click(
      screen.getByRole("button", {
        name: new RegExp(`remove participant ${MAX_PARTICIPANTS}`, "i"),
      }),
    );
    expect(
      screen.queryByRole("combobox", {
        name: `Participant ${MAX_PARTICIPANTS} role`,
      }),
    ).not.toBeInTheDocument();
    expect(addButton()).toBeInTheDocument();
  });

  it("says a picked model runs through the claude CLI, and only when there is one to pick", async () => {
    // rc3 D9 deleted the second runtime, so the picker can no longer tell a
    // model the CLI can talk to from one it cannot — `models.native`, which was
    // that distinction, is unread. Two saved models were flagged `native = 1`
    // exactly because of their gateway, and they are now offered here like any
    // other. The dialog must name the constraint where the choice is made
    // rather than let it arrive as a spawn error.
    mockBackend();
    await openDialog();
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));
    expect(screen.getByText(/spawns through the claude CLI/i)).toBeInTheDocument();
    expect(
      screen.getByText(/Anthropic Messages API/i),
    ).toBeInTheDocument();
  });

  it("drops the CLI note when there are no models, so it cannot contradict the empty-registry hint", async () => {
    mockBackend();
    const withModels = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd, ...rest) =>
      cmd === "list_models" ? [] : withModels(cmd, ...rest),
    );
    await openDialog();
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));
    expect(screen.getByText(/No saved models yet/i)).toBeInTheDocument();
    expect(screen.queryByText(/spawns through the claude CLI/i)).toBeNull();
  });

  it("offers an on-mention role, because the user can now summon one", async () => {
    // rc3 D17. This test asserted the OPPOSITE while nothing could wake an
    // `on_mention` participant: inviting one seeded a participant the ring
    // skips and nothing ever reaches. The user naming it is what changed —
    // filtering the mode out of the dialog now hides the whole feature.
    mockBackend([
      role(),
      role({ id: 7, slug: "specialist", display_name: "SPECIALIST", participation_mode: "on_mention" }),
    ]);
    await openDialog();
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));

    const options = Array.from(roleSelect(1).querySelectorAll("option")).map(
      (o) => o.textContent,
    );
    expect(options).toContain("HANDS");
    expect(options).toContain("SPECIALIST");
  });
});

// ===========================================================================
// rc3 D12 — effort is per participant
// ===========================================================================

describe("New session dialog — per-participant effort (D12)", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("carries each row's own effort into that row's payload entry", async () => {
    // The effort section used to be two fixed blocks labelled Brian and Rain.
    // The pick now belongs to the ROW, and what this asserts is the CHAIN from
    // a row's select to the entry it lands in — not the select and the payload
    // as two separate facts.
    mockBackend();
    await openDialog();
    fireEvent.change(screen.getByPlaceholderText(/refactor auth flow/i), {
      target: { value: "a task" },
    });
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));

    fireEvent.change(roleSelect(1), { target: { value: "1" } });
    fireEvent.click(screen.getByRole("button", { name: /add participant/i }));
    fireEvent.change(roleSelect(2), { target: { value: "2" } });

    // Two DIFFERENT values, so a row wired to the other row's state fails.
    fireEvent.change(effortSelect(1), { target: { value: "max" } });
    fireEvent.change(effortSelect(2), { target: { value: "low" } });

    fireEvent.click(createButton());
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("create_session", expect.anything()),
    );
    expect(sentOptions().participants).toEqual([
      { roleId: 1, modelId: null, effort: "max", ultracode: null, color: null },
      { roleId: 2, modelId: null, effort: "low", ultracode: null, color: null },
    ]);
    // The per-slot columns spawn still reads are a projection of those same
    // rows, so they cannot disagree with the roster they came from.
    expect(sentOptions().brianEffort).toBe("max");
    expect(sentOptions().rainEffort).toBe("low");
  });

  it("carries a row's ultracode tick into that row's payload entry", async () => {
    mockBackend();
    await openDialog();
    fireEvent.change(screen.getByPlaceholderText(/refactor auth flow/i), {
      target: { value: "a task" },
    });
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));

    fireEvent.change(roleSelect(1), { target: { value: "1" } });
    fireEvent.click(ultracodeBox(1));

    fireEvent.click(createButton());
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("create_session", expect.anything()),
    );
    expect(sentOptions().participants).toEqual([
      { roleId: 1, modelId: null, effort: null, ultracode: true, color: null },
    ]);
    expect(sentOptions().brianUltracode).toBe(true);
  });

  it("offers ultracode only to a role that can edit files", async () => {
    // Ultracode rides in on `--settings`, which spawn injects only on the
    // `edit_files` branch. Gating on the ticked box rather than on slot
    // position is the same rule D11 uses: capability, never role meaning.
    mockBackend([
      role(), // id 1 — read_channel + edit_files
      role({ id: 9, slug: "watcher", display_name: "WATCHER", capabilities: ["read_channel"] }),
    ]);
    await openDialog();
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));

    fireEvent.change(roleSelect(1), { target: { value: "1" } });
    expect(ultracodeBox(1)).toBeEnabled();

    fireEvent.change(roleSelect(1), { target: { value: "9" } });
    expect(ultracodeBox(1)).toBeDisabled();
  });

  it("keeps max and ultracode mutually exclusive within a row", async () => {
    mockBackend();
    await openDialog();
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));

    fireEvent.change(roleSelect(1), { target: { value: "1" } });
    fireEvent.click(ultracodeBox(1));
    expect(ultracodeBox(1)).toBeChecked();

    // Picking `max` clears the conflicting tick rather than sending both.
    fireEvent.change(effortSelect(1), { target: { value: "max" } });
    expect(ultracodeBox(1)).not.toBeChecked();
    expect(ultracodeBox(1)).toBeDisabled();
  });
});

// ===========================================================================
// rc3 D11 — the capability warning
// ===========================================================================

describe("New session dialog — capability warning (D11)", () => {
  beforeEach(() => mockInvoke.mockReset());

  /** A role with no write capability ticked. */
  const READ_ONLY = role({
    id: 5,
    slug: "eyes",
    display_name: "EYES",
    capabilities: ["read_channel", "post_channel"],
  });

  it("warns when no participant can edit files — including two of the same role", async () => {
    // The user's framing: bot-hq must not know that EYES are reviewers. It
    // knows only the ticked boxes, so it names what the UNION cannot do.
    // Duplicate roles are NOT special-cased; this is simply one roster whose
    // union is missing `edit_files`.
    mockBackend([READ_ONLY]);
    await openDialog();
    fireEvent.change(screen.getByPlaceholderText(/refactor auth flow/i), {
      target: { value: "a task" },
    });
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));

    fireEvent.change(roleSelect(1), { target: { value: "5" } });
    fireEvent.click(screen.getByRole("button", { name: /add participant/i }));
    fireEvent.change(roleSelect(2), { target: { value: "5" } });

    const warning = screen.getByRole("status");
    expect(warning).toHaveTextContent(/no participant can edit files/i);
    expect(warning).toHaveTextContent(/review, but nothing in it can act/i);
    // It says what the SET cannot do — never who the roles are.
    expect(warning.textContent).not.toMatch(/reviewer|EYES|HANDS/i);
  });

  it("does not block Create", async () => {
    mockBackend([READ_ONLY]);
    await openDialog();
    fireEvent.change(screen.getByPlaceholderText(/refactor auth flow/i), {
      target: { value: "a task" },
    });
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));
    fireEvent.change(roleSelect(1), { target: { value: "5" } });

    expect(screen.getByRole("status")).toBeInTheDocument();
    expect(createButton()).toBeEnabled();
    fireEvent.click(createButton());
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("create_session", expect.anything()),
    );
  });

  it("clears once ONE participant holds edit_files", async () => {
    mockBackend([role(), READ_ONLY]);
    await openDialog();
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));

    fireEvent.change(roleSelect(1), { target: { value: "5" } });
    expect(screen.getByRole("status")).toHaveTextContent(
      /no participant can edit files/i,
    );

    // The union is what is checked, so one editing participant is enough.
    fireEvent.click(screen.getByRole("button", { name: /add participant/i }));
    fireEvent.change(roleSelect(2), { target: { value: "1" } });
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("says nothing while a row still has no role", async () => {
    // A half-picked roster has not made a statement to warn about.
    mockBackend([READ_ONLY]);
    await openDialog();
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));
    expect(screen.queryByRole("status")).toBeNull();
  });
});

// ===========================================================================
// rc3 D10 — the effort hint reads the override store by ROLE SLUG
// ===========================================================================

describe("New session dialog — inherited effort", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("hints the effort the row's ROLE inherits, and re-resolves when the role changes", async () => {
    // One chain: the row's picked role → its slug → that slug's entry in
    // `claude-overrides.json`. The store used to be keyed by two agent names,
    // which no role slug matches, so the hint silently showed `_all` for
    // everyone — the same miss `resolve_agent_overrides` had.
    mockBackend([role(), EYES], {
      overrides: { _all: { effort: "medium" }, per_role: { eyes: { effort: "max" } } },
    });
    await openDialog();
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));

    // No role picked yet → nothing role-specific to resolve, so `_all`.
    expect(inheritOption(1)).toBe("Inherit (medium)");

    fireEvent.change(roleSelect(1), { target: { value: "2" } });
    expect(inheritOption(1)).toBe("Inherit (max)");

    // The role with no entry of its own falls back to `_all`, exactly as
    // `resolve_agent_overrides` does for an unconfigured role.
    fireEvent.change(roleSelect(1), { target: { value: "1" } });
    expect(inheritOption(1)).toBe("Inherit (medium)");
  });

  it("falls through to the settings.json knob when the store says nothing", async () => {
    mockBackend([role(), EYES], { overrides: {}, knob: "high" });
    await openDialog();
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));

    fireEvent.change(roleSelect(1), { target: { value: "1" } });
    await waitFor(() => expect(inheritOption(1)).toBe("Inherit (high)"));
  });
});

// ===========================================================================
// rc3 D10 — no participant is displayed by an agent name
// ===========================================================================

describe("New session dialog — no agent names (D10)", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("names nobody Brian or Rain, anywhere in the dialog", async () => {
    mockBackend();
    await openDialog();
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));

    const dialog = screen.getByRole("dialog", { name: /new session/i });
    expect(dialog.textContent).not.toMatch(/\bbrian\b/i);
    expect(dialog.textContent).not.toMatch(/\brain\b/i);
    // …and the roles the user DID pick from are still offered by their own
    // display names, so this is not passing by rendering nothing.
    expect(dialog.textContent).toMatch(/HANDS/);
  });
});

describe("Dashboard tiles — the Quickview byline (rc3 D10)", () => {
  beforeEach(() => mockInvoke.mockReset());

  /** One session with a Quickview to attribute, plus the roster for it. */
  function mockTile(participants: unknown[], lastAuthor: string) {
    mockInvoke.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "list_sessions":
          return [
            {
              id: "s1",
              title: "Refactor auth flow",
              working_repo_path: null,
              base_repo_path: null,
              archived: false,
              created_at: "2026-08-12T00:00:00Z",
              closed_at: null,
              brian_model_at_spawn: null,
              rain_model_at_spawn: null,
              rain_enabled: true,
              last_message: "Looking at the storage layer now",
              last_author: lastAuthor,
            },
          ];
        case "list_session_participants":
          return participants;
        case "list_pending_tray":
        case "list_projects":
        case "list_models":
        case "list_roles":
          return [];
        case "get_claude_overrides":
          return {};
        case "claude_config_read":
          return { core_knobs: [] };
        // `get_app_setting` / `get_session_phase` are nullable reads; React
        // Query rejects an `undefined` result, so answer them explicitly.
        default:
          return null;
      }
    });
  }

  const ROSTER = [
    {
      id: 1,
      slug: "hands",
      role_display_name: "HANDS",
      model_display_name: "Claude Opus 5",
      turn_position: 0,
      participation_mode: "active",
      enabled: true,
    },
  ];

  it("attributes the Quickview as ROLE · Model, resolved through the session's roster", async () => {
    // Tested as ONE chain: the stored `last_author` slug goes through
    // `list_session_participants` and comes out as the tile's byline.
    // `SessionTileLoader` exists ONLY to deliver that roster to the tile, and
    // its `authorLabels` prop could be deleted with the suite still green —
    // both halves were pinned and the wire between them was not.
    mockTile(ROSTER, "hands");
    renderDashboard();

    expect(
      await screen.findByText("HANDS · Claude Opus 5"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/looking at the storage layer now/i),
    ).toBeInTheDocument();
    // The slug is an internal key; it must not reach the tile.
    expect(screen.queryByText("hands")).toBeNull();
  });

  it("does not print a legacy agent name when the roster cannot place the author", async () => {
    // Historic rows keep `author = 'brian'` forever (rc3 D10 — "brian and
    // rain's history can be legacy data"), so the tile has to stay attributed
    // without naming an agent.
    mockTile([], "brian");
    renderDashboard();

    expect(await screen.findByText("Unknown participant")).toBeInTheDocument();
    expect(screen.queryByText(/^brian$/i)).toBeNull();
  });
});

// ===========================================================================
// rc3 D20 — a participant's colour is the user's to pick
// ===========================================================================

describe("New session dialog — participant colour (D20)", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("defaults to the rotation, and carries a pick into that row's payload", async () => {
    // "Rotate" is the DEFAULT, not an absence: the rotation already guarantees
    // no two participants of one session share a hue, so a pick is a preference
    // rather than a fix. It has to reach the backend as a NAME — the palette can
    // be re-themed and a session keeps meaning what its user meant.
    mockBackend([role()]);
    await openDialog();
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));
    fireEvent.change(roleSelect(1), { target: { value: "1" } });

    // Rotate is pressed to begin with.
    expect(
      screen.getByRole("button", { name: /participant 1 colour: rotate/i }),
    ).toHaveAttribute("aria-pressed", "true");

    fireEvent.click(
      screen.getByRole("button", { name: /participant 1 colour: cyan/i }),
    );
    fireEvent.change(screen.getByPlaceholderText(/refactor auth flow/i), {
      target: { value: "coloured" },
    });
    fireEvent.click(createButton());

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "create_session",
        expect.objectContaining({
          options: expect.objectContaining({
            participants: [
              expect.objectContaining({ roleId: 1, color: "Cyan" }),
            ],
          }),
        }),
      ),
    );
  });

  it("offers every palette entry, so the picker and the rotation cannot disagree", async () => {
    // Built from PARTICIPANT_COLORS rather than its own list — a picker with a
    // hardcoded set is how a user chooses a colour nothing renders.
    mockBackend([role()]);
    await openDialog();
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));
    for (const c of PARTICIPANT_COLORS) {
      expect(
        screen.getByRole("button", {
          name: new RegExp(`participant 1 colour: ${c.name}`, "i"),
        }),
      ).toBeInTheDocument();
    }
  });
});
