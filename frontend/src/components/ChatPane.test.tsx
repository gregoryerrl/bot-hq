import { render, screen, fireEvent, act } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
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
  author = "brian",
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
  msg(2, "hello two", "text", "rain"),
  msg(3, JSON.stringify({ name: "Bash", input: { command: "ls -la" } }), "tool_use"),
];

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) => {
    if (cmd === "get_session_messages") return Promise.resolve(initialMessages);
    return Promise.resolve([]);
  }),
}));

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
  });

  it("renders the fetched history", async () => {
    renderPane();
    expect(await screen.findByText("hello one")).toBeInTheDocument();
    expect(screen.getByText("hello two")).toBeInTheDocument();
    // The tool_use row renders as a collapsed pill, not raw JSON.
    expect(screen.getByText(/Bash/)).toBeInTheDocument();
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
