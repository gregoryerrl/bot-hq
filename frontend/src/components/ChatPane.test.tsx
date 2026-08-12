import { render, screen, fireEvent, act, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { ChatPane } from "./ChatPane";
import { useChatStore } from "../stores/chat";
import type { AgentMessage } from "../lib/bindings";

// The virtualizer observes the scroll element; jsdom has no ResizeObserver.
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
vi.stubGlobal("ResizeObserver", ResizeObserverStub);

// jsdom elements measure 0×0, which makes the virtualizer compute an empty
// visible range and mount nothing. virtual-core reads the scroll viewport
// from offsetWidth/offsetHeight and row sizes from getBoundingClientRect —
// stub both so rows mount and measure.
Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
  configurable: true,
  get: () => 600,
});
Object.defineProperty(HTMLElement.prototype, "offsetWidth", {
  configurable: true,
  get: () => 800,
});
vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue({
  width: 800,
  height: 60,
  top: 0,
  left: 0,
  bottom: 60,
  right: 800,
  x: 0,
  y: 0,
  toJSON: () => ({}),
} as DOMRect);

function msg(
  id: number,
  content: string,
  kind: AgentMessage["kind"] = "text",
  author = "hands",
): AgentMessage {
  return {
    id,
    session_id: "s1",
    author,
    kind,
    content,
    created_at: "2026-07-18T00:00:00Z",
  } as AgentMessage;
}

const initialMessages: AgentMessage[] = [
  msg(1, "hello one"),
  msg(2, "hello two", "text", "eyes"),
  msg(3, JSON.stringify({ name: "Bash", input: { command: "ls -la" } }), "tool_use"),
];

/**
 * The session's roster, as `list_session_participants` returns it (rc3 D10).
 * Two participants sharing a role, with different models — the configuration
 * the user could not tell apart until the agents said so.
 */
const PARTICIPANTS = [
  {
    id: 1,
    slug: "hands",
    role_display_name: "HANDS",
    model_display_name: "Claude Opus 5",
    turn_position: 0,
    participation_mode: "active",
    enabled: true,
  },
  {
    id: 2,
    slug: "eyes",
    role_display_name: "EYES",
    model_display_name: "DeepSeek R2",
    turn_position: 1,
    participation_mode: "active",
    enabled: true,
  },
];

/** The default backend for these tests; restored before each one so a test
 *  that swaps it (to stage a different roster) cannot leak into the next. */
const defaultInvoke = (cmd: string) => {
  if (cmd === "get_session_messages") return Promise.resolve(initialMessages);
  if (cmd === "list_session_participants") return Promise.resolve(PARTICIPANTS);
  return Promise.resolve([]);
};

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

// Capture event handlers so tests can push live batches like the backend does.
const eventHandlers: Record<string, (ev: { payload: unknown }) => void> = {};
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (ev: { payload: unknown }) => void) => {
    eventHandlers[name] = cb;
    return Promise.resolve(() => {
      delete eventHandlers[name];
    });
  }),
}));

function renderPane() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <ChatPane sessionId="s1" />
    </QueryClientProvider>,
  );
}

describe("ChatPane", () => {
  beforeEach(() => {
    // The zustand store is module-global — reset between tests.
    useChatStore.setState({ messages: {}, watermarks: {} });
    vi.mocked(invoke).mockImplementation(defaultInvoke);
  });

  it("renders the fetched history", async () => {
    renderPane();
    expect(await screen.findByText("hello one")).toBeInTheDocument();
    expect(screen.getByText("hello two")).toBeInTheDocument();
    // The tool_use row renders as a collapsed pill, not raw JSON.
    expect(screen.getByText(/Bash/)).toBeInTheDocument();
  });

  it("bylines each message as ROLE · Model, resolved through the roster", async () => {
    // rc3 D10, tested as ONE chain: the stored `author` slug goes through
    // `list_session_participants` and comes out as the rendered byline. Pinning
    // the lookup and the render separately would not catch a pane that renders
    // the slug it holds instead of the label it resolved.
    renderPane();
    await screen.findByText("hello one");

    // Two rows carry the `hands` slug (a text message and a tool call), so
    // findAll — one byline per ungrouped header.
    expect(
      (await screen.findAllByText("HANDS · Claude Opus 5")).length,
    ).toBeGreaterThan(0);
    expect(screen.getByText("EYES · DeepSeek R2")).toBeInTheDocument();
    // The slug is an internal key; it must not reach the screen.
    expect(screen.queryByText("hands")).toBeNull();
    expect(screen.queryByText("eyes")).toBeNull();
  });

  it("keeps two participants of the SAME role apart by their models", async () => {
    // The live complaint: "I accidentally set the two agents to EYES + EYES,
    // there's no way for me to know that until they explicitly said in the
    // session". Same role, different model — the byline has to distinguish them.
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_session_messages")
        return Promise.resolve([
          msg(1, "first reviewer speaks", "text", "eyes"),
          msg(2, "second reviewer speaks", "text", "eyes-2"),
        ]);
      if (cmd === "list_session_participants")
        return Promise.resolve([
          { ...PARTICIPANTS[1], id: 1, slug: "eyes", model_display_name: "Claude Opus 5" },
          { ...PARTICIPANTS[1], id: 2, slug: "eyes-2", model_display_name: "DeepSeek R2" },
        ]);
      return Promise.resolve([]);
    });

    renderPane();
    await screen.findByText("first reviewer speaks");
    expect(await screen.findByText("EYES · Claude Opus 5")).toBeInTheDocument();
    expect(screen.getByText("EYES · DeepSeek R2")).toBeInTheDocument();
  });

  it("falls back to the author slug when the roster has no row for it", async () => {
    // A legacy row, or a participant that has left: the line still has to be
    // attributable, and an internal key beats no attribution at all.
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_session_messages")
        return Promise.resolve([msg(1, "orphan line", "text", "departed")]);
      if (cmd === "list_session_participants") return Promise.resolve([]);
      return Promise.resolve([]);
    });

    renderPane();
    expect(await screen.findByText("departed")).toBeInTheDocument();
  });

  it("appends live batches for this session and ignores other sessions", async () => {
    renderPane();
    await screen.findByText("hello one");
    act(() => {
      eventHandlers["agent:messages:batch"]?.({
        payload: [
          msg(4, "late message"),
          { ...msg(5, "foreign message"), session_id: "OTHER" },
        ],
      });
    });
    expect(await screen.findByText("late message")).toBeInTheDocument();
    expect(screen.queryByText("foreign message")).not.toBeInTheDocument();
  });

  it("marks a tool call running until its result lands", async () => {
    // A five-minute `cargo build --release` and a 20ms `Read` used to render
    // identically, with nothing on screen changing while the build ran.
    renderPane();
    await screen.findByText("hello one");
    act(() => {
      eventHandlers["agent:messages:batch"]?.({
        payload: [
          {
            ...msg(
              10,
              JSON.stringify({
                name: "Bash",
                input: { command: "cargo build --release" },
                tool_use_id: "t-run",
              }),
              "tool_use",
            ),
            // Elapsed is derived from created_at, so a just-started call needs a
            // current stamp (the shared fixture's is weeks old on purpose).
            created_at: new Date().toISOString(),
          },
        ],
      });
    });
    // Running: the ⟳ glyph replaces →, and an elapsed counter appears.
    expect(await screen.findByText(/⟳ Bash/)).toBeInTheDocument();
    expect(screen.getByText(/^\d+s$/)).toBeInTheDocument();

    act(() => {
      eventHandlers["agent:messages:batch"]?.({
        payload: [
          msg(
            11,
            JSON.stringify({ tool_use_id: "t-run", output: "Finished" }),
            "tool_result",
          ),
        ],
      });
    });
    // Resolved: the running marker is gone and the row joins the seeded Bash
    // call in the plain → form (hence findAll — two Bash rows by now).
    await waitFor(() =>
      expect(screen.queryByText(/⟳ Bash/)).not.toBeInTheDocument(),
    );
    expect(screen.getAllByText(/→ Bash/).length).toBe(2);
  });

  it("does not claim a tool is running when it carries no tool_use_id", async () => {
    // The seeded row has no `tool_use_id`, so nothing can match it to a result.
    // Guessing "running" there would be a fresh lie, not a fix.
    renderPane();
    await screen.findByText("hello one");
    expect(await screen.findByText(/→ Bash/)).toBeInTheDocument();
    expect(screen.queryByText(/⟳/)).not.toBeInTheDocument();
  });

  it("shows the full command in the collapsed pill, not an 80-char clip", async () => {
    renderPane();
    await screen.findByText("hello one");
    const long =
      "cargo test > /tmp/gate1.log 2>&1; echo \"cargo test exit=$?\"; tail -25 /tmp/gate1.log";
    act(() => {
      eventHandlers["agent:messages:batch"]?.({
        payload: [
          msg(
            12,
            JSON.stringify({
              name: "Bash",
              input: { command: long },
              tool_use_id: "t-long",
            }),
            "tool_use",
          ),
        ],
      });
    });
    expect(await screen.findByText(long)).toBeInTheDocument();
  });

  it("expands and collapses a tool pill via the lifted state", async () => {
    renderPane();
    await screen.findByText("hello one");
    const pill = screen.getByRole("button", { expanded: false });
    fireEvent.click(pill);
    // Expanded body renders the pretty-printed JSON payload.
    expect(await screen.findByText(/"ls -la"/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { expanded: true }));
    expect(screen.queryByText(/"ls -la"/)).not.toBeInTheDocument();
  });
});
