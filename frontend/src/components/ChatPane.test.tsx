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

function renderPane(onViewFile?: (path: string) => void) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <ChatPane sessionId="s1" onViewFile={onViewFile} />
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

  it("keeps two participants of the SAME role apart", async () => {
    // The live complaint: "I accidentally set the two agents to EYES + EYES,
    // there's no way for me to know that until they explicitly said in the
    // session". Same role — the byline has to distinguish them.
    //
    // Two things do it now. The model differs here, which was the original fix;
    // rc3 D20 added the ORDINAL, which is what covers the harder case the user
    // hit later — same role AND same model, where the model told them nothing:
    // "for the 2 reviewers, i don't know which is which".
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
    expect(screen.getByText("EYES-2 · DeepSeek R2")).toBeInTheDocument();
  });

  it("keeps them apart even on the SAME model, which the byline could not before", async () => {
    // rc3 D20, and the case that has no other signal: one role, one model, two
    // participants. Before the ordinal both bylines read `EYES · DeepSeek V4
    // Pro` — identical strings, and `authorColor` hashes the string, so
    // identical colours too.
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_session_messages")
        return Promise.resolve([
          msg(1, "first reviewer speaks", "text", "eyes"),
          msg(2, "second reviewer speaks", "text", "eyes-2"),
        ]);
      if (cmd === "list_session_participants")
        return Promise.resolve([
          { ...PARTICIPANTS[1], id: 1, slug: "eyes", model_display_name: "DeepSeek V4 Pro" },
          { ...PARTICIPANTS[1], id: 2, slug: "eyes-2", model_display_name: "DeepSeek V4 Pro" },
        ]);
      return Promise.resolve([]);
    });

    renderPane();
    await screen.findByText("first reviewer speaks");
    expect(await screen.findByText("EYES · DeepSeek V4 Pro")).toBeInTheDocument();
    expect(screen.getByText("EYES-2 · DeepSeek V4 Pro")).toBeInTheDocument();
  });

  it("attributes a row the roster cannot back WITHOUT printing its slug", async () => {
    // A legacy row, or a participant that has left. rc3 D10 keeps these
    // renderable on purpose — "brian and rain's history can be legacy data" —
    // and every legacy row carries `author = 'brian'` / `'rain'`, so printing
    // the slug here put the two removed names straight back on screen.
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_session_messages")
        return Promise.resolve([msg(1, "orphan line", "text", "brian")]);
      if (cmd === "list_session_participants") return Promise.resolve([]);
      return Promise.resolve([]);
    });

    renderPane();
    // The line survives, attributed…
    expect(await screen.findByText("orphan line")).toBeInTheDocument();
    expect(await screen.findByText("Unknown participant")).toBeInTheDocument();
    // …and the stored agent name does not reach the screen.
    expect(screen.queryByText(/^brian$/i)).toBeNull();
  });

  it("never renders a BLANK byline, even for a row with no author at all", async () => {
    // `authorLabel` answers "" for an empty author, which is the one input that
    // reaches the header's own fallback. An empty byline would read as part of
    // the previous message rather than as an unattributed one.
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_session_messages")
        return Promise.resolve([msg(1, "authorless line", "text", "")]);
      if (cmd === "list_session_participants") return Promise.resolve([]);
      return Promise.resolve([]);
    });

    renderPane();
    expect(await screen.findByText("authorless line")).toBeInTheDocument();
    expect(screen.getByText("Unknown participant")).toBeInTheDocument();
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

  it("mounts on the newest page and loads older rows on demand (round 8, N2)", async () => {
    // A 3,412-row session used to be pulled whole across IPC on every mount.
    // The mount read asks for CHAT_PAGE + 1 (the extra row is a PROBE, trimmed
    // before the store sees it); a full probe shows the "Load older" button;
    // the next page is asked for BEFORE the oldest held id and is prepended,
    // and a short probe hides the button. Exactly 2 × CHAT_PAGE rows is the
    // case a "full page ⇒ more" heuristic got wrong (one dud click); the
    // probe gets it right: two pages, then no button.
    const { CHAT_PAGE } = await import("./ChatPane");
    const row = (id: number) => ({ ...msg(id, `row ${id}`, "text"), id });
    const total = CHAT_PAGE * 2;
    const calls: Array<Record<string, unknown> | undefined> = [];
    vi.mocked(invoke).mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "get_session_messages") {
        const a = args as { limit?: number; beforeId?: number | null } | undefined;
        calls.push(a);
        const before = a?.beforeId ?? total + 1;
        const limit = a?.limit ?? total;
        const ids = [];
        for (let id = before - 1; id >= 1 && ids.length < limit; id--) ids.push(id);
        return Promise.resolve(ids.reverse().map(row));
      }
      if (cmd === "list_session_participants") return Promise.resolve(PARTICIPANTS);
      return Promise.resolve([]);
    });
    renderPane();
    // The mount read asks for the probe…
    await waitFor(() => expect(calls[0]?.limit).toBe(CHAT_PAGE + 1));
    const older = await screen.findByRole("button", { name: /older/i });
    // …and the store holds exactly one page (probe trimmed), ending at the newest row.
    expect(useChatStore.getState().messages["s1"]).toHaveLength(CHAT_PAGE);
    expect(useChatStore.getState().messages["s1"]?.[CHAT_PAGE - 1].id).toBe(total);
    // The button describes the action, not a count it cannot know.
    expect(older.textContent).toBe(`Load ${CHAT_PAGE} older`);
    fireEvent.click(older);
    await waitFor(() =>
      expect(useChatStore.getState().messages["s1"]).toHaveLength(CHAT_PAGE * 2),
    );
    // Asked for the page BEFORE the oldest held id, with the probe, prepended in order.
    expect(calls[1]?.beforeId).toBe(total - CHAT_PAGE + 1);
    expect(calls[1]?.limit).toBe(CHAT_PAGE + 1);
    expect(useChatStore.getState().messages["s1"]?.[0].id).toBe(1);
    // Exactly two pages existed: the second probe came back short, so the
    // button is gone without a dud click.
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: /older/i })).not.toBeInTheDocument(),
    );
  });

  it("offers to view the file a tool call names — from its args, never its prose (round 8)", async () => {
    // issues.md #1's second half: the file viewer existed and was wired to the
    // gate card only. A Read's `file_path`, a Bash file argument, get a View
    // button; a command with no file argument, and prose, do not.
    const onViewFile = vi.fn();
    renderPane(onViewFile);
    await screen.findByText("hello one");
    act(() => {
      eventHandlers["agent:messages:batch"]?.({
        payload: [
          msg(
            20,
            JSON.stringify({
              name: "Read",
              input: { file_path: "/repo/src/lib.rs" },
              tool_use_id: "t-read",
            }),
            "tool_use",
          ),
          msg(
            21,
            JSON.stringify({
              name: "Bash",
              input: { command: "cat --file /tmp/pr-body.md" },
              tool_use_id: "t-cat",
            }),
            "tool_use",
          ),
          msg(22, "see /repo/src/lib.rs for details", "text"),
        ],
      });
    });
    const readBtn = await screen.findByRole("button", { name: "View lib.rs" });
    fireEvent.click(readBtn);
    expect(onViewFile).toHaveBeenCalledWith("/repo/src/lib.rs");
    fireEvent.click(screen.getByRole("button", { name: "View pr-body.md" }));
    expect(onViewFile).toHaveBeenCalledWith("/tmp/pr-body.md");
    // The seeded `ls -la` Bash row and the prose row offer nothing.
    expect(screen.getAllByRole("button", { name: /^View / })).toHaveLength(2);
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
