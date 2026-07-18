import { render, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { SessionTerminalTab } from "./SessionTerminalTab";

// xterm needs a real canvas — mock the whole module and observe the calls.
const termInstance = {
  loadAddon: vi.fn(),
  open: vi.fn(),
  write: vi.fn((_d: unknown, cb?: () => void) => cb?.()),
  writeln: vi.fn(),
  onData: vi.fn((_cb: (data: string) => void) => ({ dispose: vi.fn() })),
  dispose: vi.fn(),
  cols: 80,
  rows: 24,
};
vi.mock("@xterm/xterm", () => ({
  Terminal: vi.fn().mockImplementation(() => termInstance),
}));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: vi.fn().mockImplementation(() => ({ fit: vi.fn() })),
}));

const invokeMock = vi.fn((cmd: string, _args?: unknown) => {
  switch (cmd) {
    case "terminal_open":
      return Promise.resolve({
        snapshot_b64: btoa("replayed-history"),
        cols: 120,
        rows: 30,
      });
    default:
      return Promise.resolve(null);
  }
});
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

// Capture event handlers by name so tests can fire terminal:output.
const listeners: Record<string, (e: { payload: unknown }) => void> = {};
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) => {
    listeners[name] = cb;
    return Promise.resolve(() => {});
  }),
}));

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", ResizeObserverStub);
  invokeMock.mockClear();
  termInstance.write.mockClear();
  termInstance.onData.mockClear();
});

const decode = (bytes: Uint8Array) => new TextDecoder().decode(bytes);

describe("SessionTerminalTab", () => {
  it("opens the PTY and replays the scrollback snapshot", async () => {
    render(<SessionTerminalTab sessionId="s1" active={true} />);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("terminal_open", {
        sessionId: "s1",
      }),
    );
    await waitFor(() => expect(termInstance.write).toHaveBeenCalled());
    const first = termInstance.write.mock.calls[0][0] as Uint8Array;
    expect(decode(first)).toBe("replayed-history");
  });

  it("forwards keystrokes through terminal_input", async () => {
    render(<SessionTerminalTab sessionId="s1" active={true} />);
    await waitFor(() => expect(termInstance.onData).toHaveBeenCalled());
    const onData = termInstance.onData.mock.calls[0][0] as (d: string) => void;
    onData("ls\r");
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("terminal_input", {
        sessionId: "s1",
        data: "ls\r",
      }),
    );
  });

  it("writes terminal:output events for this session only, after replay", async () => {
    render(<SessionTerminalTab sessionId="s1" active={true} />);
    await waitFor(() => expect(termInstance.write).toHaveBeenCalled());
    termInstance.write.mockClear();

    listeners["terminal:output"]({
      payload: { session_id: "other", data: btoa("nope"), seq: 1 },
    });
    expect(termInstance.write).not.toHaveBeenCalled();

    listeners["terminal:output"]({
      payload: { session_id: "s1", data: btoa("live-chunk"), seq: 2 },
    });
    await waitFor(() => expect(termInstance.write).toHaveBeenCalled());
    const written = termInstance.write.mock.calls[0][0] as Uint8Array;
    expect(decode(written)).toBe("live-chunk");
  });
});
