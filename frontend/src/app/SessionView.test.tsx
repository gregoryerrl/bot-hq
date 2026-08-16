import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { SessionView } from "./SessionView";
import { useActivityStore } from "../stores/activity";
import { useHealthStore } from "../stores/health";
import { useContextStore } from "../stores/context";
import { slotKey } from "../lib/participants";

// The Terminal panel mounts the real SessionTerminalTab on pill click — mock
// xterm out (jsdom has no matchMedia/canvas); panel-switching is what's under
// test here, not the terminal (SessionTerminalTab.test.tsx covers that).
vi.mock("@xterm/xterm", () => ({
  Terminal: vi.fn().mockImplementation(() => ({
    loadAddon: vi.fn(),
    open: vi.fn(),
    write: vi.fn((_d: unknown, cb?: () => void) => cb?.()),
    writeln: vi.fn(),
    onData: vi.fn((_cb: (data: string) => void) => ({ dispose: vi.fn() })),
    dispose: vi.fn(),
    cols: 80,
    rows: 24,
  })),
}));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: vi.fn().mockImplementation(() => ({ fit: vi.fn() })),
}));
// jsdom has no WebGL2 — stub the WebGL addon (no-op) so clicking the Terminal
// pill doesn't run the real renderer and log a context-creation error.
vi.mock("@xterm/addon-webgl", () => ({
  WebglAddon: vi.fn().mockImplementation(() => ({
    onContextLoss: vi.fn(),
    dispose: vi.fn(),
  })),
}));

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
vi.stubGlobal("ResizeObserver", ResizeObserverStub);

// rc3 D10: the header roster. Two rows sharing a role is the exact
// configuration the user could not see until the agents said so. Mutable so a
// test can hand the view a roster whose SPAWNABLE order differs from its turn
// positions — see the disabled-row test below.
const roster = vi.hoisted(() => ({
  rows: [] as unknown[],
  default: [
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
  ] as unknown[],
}));

// rc3 P1: what `get_participant_system_prompt` answers, and what it was asked.
// Mutable so one test can hand back a prompt and another an absence, and so the
// slug the view sent is assertable — attribution is the half of P1 that a
// rendered string alone does not pin.
const promptCall = vi.hoisted(() => ({
  reply: {} as Record<string, unknown>,
  lastArgs: null as Record<string, unknown> | null,
}));

// rc3 P7: the recorded context readings a participant's meter opens onto.
const readingsCall = vi.hoisted(() => ({
  rows: [] as unknown[],
  lastArgs: null as Record<string, unknown> | null,
}));

// Keyed invoke mock: `get_session` must return a session row or the view
// renders its not-found state; everything else gets an empty default.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case "get_participant_system_prompt":
        promptCall.lastArgs = args ?? null;
        return Promise.resolve(promptCall.reply);
      case "list_participant_context_readings":
        readingsCall.lastArgs = args ?? null;
        return Promise.resolve(readingsCall.rows);
      case "terminal_open":
        return Promise.resolve({ snapshot_b64: "", cols: 80, rows: 24 });
      case "get_session":
        return Promise.resolve({
          id: "s1",
          title: "Subtab test session",
          working_repo_path: null,
          base_repo_path: null,
          archived: false,
          created_at: "2026-07-18T00:00:00Z",
          closed_at: null,
          brian_model_at_spawn: null,
          rain_model_at_spawn: null,
          rain_enabled: true,
        });
      case "get_session_phase":
        return Promise.resolve(null);
      case "list_session_participants":
        return Promise.resolve(roster.rows);
      case "compute_apply_diff":
        return Promise.resolve({ files: [], truncated: false });
      default:
        return Promise.resolve([]);
    }
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

function renderSessionView() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={["/session/s1"]}>
        <Routes>
          <Route path="/session/:sessionId" element={<SessionView />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

// jsdom loads no Tailwind CSS, so the `hidden` utility has no computed
// effect — panel visibility is asserted via the class list instead.
function panel(name: string): HTMLElement {
  return screen.getByRole("tabpanel", { name });
}

// The zustand runtime stores are module-global — reset between tests so a
// seeded slot/slug key cannot leak into a test that assumes nothing reported.
beforeEach(() => {
  useActivityStore.setState({ bySession: {}, busyBySession: {} });
  useHealthStore.setState({ bySession: {} });
  useContextStore.setState({ bySession: {} });
  roster.rows = roster.default;
  promptCall.reply = {};
  promptCall.lastArgs = null;
  readingsCall.rows = [];
  readingsCall.lastArgs = null;
});

/** The worker's own line in the turn-status row: `<label> is working`. */
function workerLine(): string {
  return screen.getByText("is working").parentElement!.textContent!;
}

describe("SessionView turn-status line (rc3 D10)", () => {
  it("names the working participant as ROLE · Model, resolved through the roster", async () => {
    // Tested as ONE chain: the busy map's key goes through
    // `list_session_participants` and comes out as the rendered status line.
    // Pinning `ChatInput`'s rendering and the roster lookup separately would
    // not catch a SessionView that stops handing the resolver down — which is
    // exactly the state this test was written against (the whole `busyLabel`
    // prop could be deleted with the suite still green).
    //
    // The key is a SLOT key because that is what `session:activity` produces:
    // `brian_busy` / `rain_busy` are frozen wire naming turn slots 0 and 1
    // (`src/core/activity.rs` fills them from `slugs.get(0)` / `.get(1)`).
    useActivityStore.setState({
      bySession: { s1: "busy" },
      busyBySession: { s1: { [slotKey(0)]: true } },
    });
    renderSessionView();
    await screen.findByRole("button", { name: "Workspace" });

    expect(workerLine()).toBe("EYES · Claude Opus 5is working");
  });

  it("says the participant is unknown rather than printing the internal key", async () => {
    // A busy flag for a slot the roster cannot account for. Printing the raw
    // key here is what put "brian is working" on screen, because the frontend
    // used to unpack the frozen pair under the literal slugs `brian` / `rain`.
    useActivityStore.setState({
      bySession: { s1: "busy" },
      busyBySession: { s1: { brian: true } },
    });
    renderSessionView();
    await screen.findByRole("button", { name: "Workspace" });

    expect(workerLine()).toBe("Unknown participantis working");
    expect(workerLine()).not.toMatch(/brian/i);
  });
});

describe("SessionView health dots + context meters (rc3 D10)", () => {
  it("finds a participant's runtime state under EITHER key the backend emits", async () => {
    // The two halves disagreed silently. `get_session_runtime`'s
    // `brian_health` / `rain_health` are SLOT-shaped (filled from
    // `handle.participants.get(0)` / `.get(1)`), while the live
    // `session:agent_health` event keys by the participant's own slug. A
    // lookup keyed to one space alone leaves the other blank — and a blank dot
    // renders as a healthy one, so nothing looks wrong.
    useHealthStore.setState({
      // Slot-keyed, as the mount backfill seeds it.
      // Slug-keyed, as the live event seeds it.
      bySession: { s1: { [slotKey(0)]: "stalled", "eyes-2": "dead" } },
    });
    useContextStore.setState({
      bySession: {
        s1: {
          // Slug-keyed, as the live event seeds it.
          eyes: { usedTokens: 620_000, contextWindow: 1_000_000 },
          // Slot-keyed, as the backfill seeds it — and the SECOND slot, so
          // resolving it needs this row's place in the roster, not the row.
          [slotKey(1)]: { usedTokens: 310_000, contextWindow: 1_000_000 },
        },
      },
    });
    renderSessionView();
    await screen.findByRole("button", { name: "Workspace" });

    expect(
      screen.getByLabelText(
        "EYES · Claude Opus 5 stalled — no response, possibly hung",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText("EYES-2 · DeepSeek R2 stopped — gave up after errors"),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText("EYES · Claude Opus 5 context 62 percent used"),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText("EYES-2 · DeepSeek R2 context 31 percent used"),
    ).toBeInTheDocument();
  });

  it("leaves a participant nothing reported for reading as healthy, not as another's state", async () => {
    // Slot 1 has no entry in either space; it must not inherit slot 0's.
    useHealthStore.setState({ bySession: { s1: { [slotKey(0)]: "dead" } } });
    renderSessionView();
    await screen.findByRole("button", { name: "Workspace" });

    expect(
      screen.getByLabelText("EYES · Claude Opus 5 stopped — gave up after errors"),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText("EYES-2 · DeepSeek R2 running"),
    ).toBeInTheDocument();
  });

  it("attributes a slot to the row that RUNS in it, not the row at that turn position", async () => {
    // `get_session_runtime` fills its pair from `handle.participants.get(0)`,
    // and `handle.participants` is `spawnable(roster)` — the enabled,
    // non-on-demand rows. A disabled row still holds turn position 0, so slot 0
    // here belongs to the row at turn position 1. Keying the lookup off
    // `turn_position` showed the running participant's health on the row that
    // is not running, and left the running one blank.
    roster.rows = [
      { ...(roster.default[0] as object), enabled: false },
      roster.default[1],
    ];
    useHealthStore.setState({ bySession: { s1: { [slotKey(0)]: "dead" } } });
    renderSessionView();
    await screen.findByRole("button", { name: "Workspace" });

    expect(
      screen.getByLabelText("EYES-2 · DeepSeek R2 stopped — gave up after errors"),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText("EYES · Claude Opus 5 running"),
    ).toBeInTheDocument();
  });
});

describe("SessionView header roster (rc3 D10)", () => {
  it("names every participant as ROLE · Model while the session runs", async () => {
    // The user's live complaint: "I accidentally set the two agents to EYES +
    // EYES, there's no way for me to know that until they explicitly said in
    // the session, so I forced closed the session." The composition has to be
    // legible from the header, not inferred from what the agents say.
    renderSessionView();
    await screen.findByRole("button", { name: "Workspace" });

    const header = screen.getByRole("banner");
    expect(header).toHaveTextContent("EYES · Claude Opus 5");
    expect(header).toHaveTextContent("EYES-2 · DeepSeek R2");
    // Slugs are internal keys — the header must not print them.
    expect(header.textContent).not.toMatch(/eyes-2/);
    // …and no participant is named after an agent (rc3 D10).
    expect(header.textContent).not.toMatch(/\bbrian\b/i);
    expect(header.textContent).not.toMatch(/\brain\b/i);
  });
});

describe("SessionView subtabs", () => {
  it("renders the Workspace | Context | Terminal pill row", async () => {
    renderSessionView();
    expect(
      await screen.findByRole("button", { name: "Workspace" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Context" })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Terminal" }),
    ).toBeInTheDocument();
  });

  it("shows Workspace by default and hides the other panels", async () => {
    renderSessionView();
    await screen.findByRole("button", { name: "Workspace" });
    expect(panel("Workspace").className).not.toContain("hidden");
    expect(panel("Context").className).toContain("hidden");
    expect(panel("Terminal").className).toContain("hidden");
    // Workspace content is intact: chat input + pane splitter.
    expect(
      screen.getByPlaceholderText("Broadcast to every participant…"),
    ).toBeInTheDocument();
    expect(screen.getByRole("separator")).toBeInTheDocument();
  });

  it("switches panels on pill click without unmounting the others", async () => {
    renderSessionView();
    fireEvent.click(await screen.findByRole("button", { name: "Context" }));
    expect(panel("Context").className).not.toContain("hidden");
    expect(panel("Workspace").className).toContain("hidden");
    // Keep-mounted: the workspace chat input is still in the DOM while hidden.
    expect(
      screen.getByPlaceholderText("Broadcast to every participant…"),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Terminal" }));
    expect(panel("Terminal").className).not.toContain("hidden");
    expect(panel("Context").className).toContain("hidden");

    fireEvent.click(screen.getByRole("button", { name: "Workspace" }));
    expect(panel("Workspace").className).not.toContain("hidden");
    expect(panel("Terminal").className).toContain("hidden");
  });
});

describe("SessionView composed system prompt (rc3 P1)", () => {
  it("opens the prompt of the participant whose chip was clicked", async () => {
    // The defect P1 closes is that ~48 KB of standing instruction — appended
    // to claude-code's own system prompt — was visible to nobody. Asserted as
    // ONE chain: the chip's slug goes out on the invoke and the answer comes
    // back under that participant's heading. Two participants share a ROLE
    // here, which is exactly the roster where showing the wrong one would look
    // right.
    promptCall.reply = {
      slug: "eyes-2",
      content: "SENTINEL_PROMPT_BODY_Q4\n\n## Capabilities",
      bytes: 41,
      unavailable: null,
    };
    renderSessionView();
    fireEvent.click(
      await screen.findByRole("button", { name: "EYES-2 · DeepSeek R2" }),
    );

    const dialog = await screen.findByRole("dialog", {
      name: "EYES-2 · DeepSeek R2 — composed system prompt",
    });
    expect(promptCall.lastArgs).toEqual({ sessionId: "s1", slug: "eyes-2" });
    expect(dialog).toHaveTextContent("SENTINEL_PROMPT_BODY_Q4");
    // The append caveat is stated every time, and NOT inside the prompt body:
    // pasted into the text it would read as one more instruction the agent got.
    expect(dialog).toHaveTextContent(/APPENDS this to claude-code's own system prompt/);
    expect(
      dialog.querySelector("pre")?.textContent,
    ).not.toMatch(/APPENDS/);
  });

  it("shows why a prompt is missing instead of an empty pane", async () => {
    // A blank viewer teaches the user that the prompt is empty, which is the
    // opposite of what this view exists to show.
    promptCall.reply = {
      slug: "eyes",
      content: null,
      bytes: 0,
      unavailable: "This session has no live agents.",
    };
    renderSessionView();
    fireEvent.click(
      await screen.findByRole("button", { name: "EYES · Claude Opus 5" }),
    );

    const dialog = await screen.findByRole("dialog", {
      name: "EYES · Claude Opus 5 — composed system prompt",
    });
    expect(dialog).toHaveTextContent("This session has no live agents.");
    // No byte count and no append caveat: there is no prompt to describe.
    expect(dialog).not.toHaveTextContent(/APPENDS/);
  });
});

describe("SessionView context readings (rc3 P7)", () => {
  it("offers the recorded readings even when no meter is showing", async () => {
    // The state the 2026-08-12 death happened in: a provider that reports no
    // `contextWindow`, so the meter renders nothing and nobody could have been
    // warned. The badge is still reachable, and the history explains WHY there
    // is no percentage — that is the whole point of persisting the absences.
    readingsCall.rows = [
      {
        model: null,
        used_tokens: null,
        reported_window: null,
        verdict: "no_window",
        created_at: "2026-08-13T00:10:00Z",
      },
      {
        model: "claude-opus-5",
        used_tokens: 620000,
        reported_window: 1000000,
        verdict: "usable",
        created_at: "2026-08-13T00:11:00Z",
      },
    ];
    renderSessionView();
    // No context store state is seeded, so no participant has a live meter.
    fireEvent.click(
      await screen.findByRole("button", {
        name: "EYES-2 · DeepSeek R2 context history",
      }),
    );

    const dialog = await screen.findByRole("dialog", {
      name: "EYES-2 · DeepSeek R2 — context readings",
    });
    expect(readingsCall.lastArgs).toEqual({ sessionId: "s1", slug: "eyes-2" });
    expect(dialog).toHaveTextContent("no_window");
    // A missing operand is printed as a dash, never as a substituted figure —
    // the meter's denominator must not be quietly back-filled from config.
    expect(dialog.querySelector("pre")?.textContent).toContain("— / —");
    expect(dialog.querySelector("pre")?.textContent).toContain(
      "620,000 / 1,000,000 (62%)",
    );
  });

  it("says so when a participant has no recorded readings", async () => {
    readingsCall.rows = [];
    renderSessionView();
    fireEvent.click(
      await screen.findByRole("button", {
        name: "EYES · Claude Opus 5 context history",
      }),
    );
    const dialog = await screen.findByRole("dialog", {
      name: "EYES · Claude Opus 5 — context readings",
    });
    expect(dialog).toHaveTextContent("No readings recorded");
  });
});
