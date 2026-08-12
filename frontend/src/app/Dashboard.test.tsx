import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { Dashboard } from "./Dashboard";
import { invoke } from "@tauri-apps/api/core";
import type { ModelView, RoleView } from "../lib/bindings";

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
    native: false,
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
    archived: false,
    ...over,
  };
}

const EYES = role({ id: 2, slug: "eyes", display_name: "EYES" });

/** Wires every read the dashboard makes; `roles` overrides the role list. */
function mockBackend(roles: RoleView[] = [role(), EYES]) {
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
        return {};
      case "claude_config_read":
        return { core_knobs: [] };
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
const createButton = () =>
  screen.getByRole("button", { name: /create session/i });

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
              { roleId: 1, modelId: null },
              { roleId: 2, modelId: "m-opus" },
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

  it("stops at the number of participants the runtime can spawn", async () => {
    mockBackend();
    await openDialog();
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));

    fireEvent.click(screen.getByRole("button", { name: /add participant/i }));
    expect(roleSelect(2)).toBeInTheDocument();
    // A third row would be a participant with no process behind it — spawn
    // starts two literally-named agents.
    expect(
      screen.queryByRole("button", { name: /add participant/i }),
    ).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /remove participant 2/i }));
    expect(
      screen.queryByRole("combobox", { name: "Participant 2 role" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /add participant/i }),
    ).toBeInTheDocument();
  });

  it("does not offer an on-demand role", async () => {
    // rc3 D1: waking one needs a user @mention, which is not built. Offering
    // it would seed a participant the ring skips and nothing ever wakes.
    mockBackend([
      role(),
      role({ id: 7, slug: "specialist", display_name: "SPECIALIST", participation_mode: "on_demand" }),
    ]);
    await openDialog();
    await waitFor(() => expect(roleSelect(1)).toHaveValue(""));

    const options = Array.from(roleSelect(1).querySelectorAll("option")).map(
      (o) => o.textContent,
    );
    expect(options).toContain("HANDS");
    expect(options).not.toContain("SPECIALIST");
  });
});
