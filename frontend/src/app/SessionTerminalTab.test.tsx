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
  onKey: vi.fn((_cb: (e: { key: string }) => void) => ({ dispose: vi.fn() })),
  dispose: vi.fn(),
  cols: 80,
  rows: 24,
};
// The component `new`s all three classes below, and since Vitest 4 a mock called
// with `new` needs a `function` (or `class`) implementation — an arrow throws
// "is not a constructor". For WebglAddon that throw would be swallowed by the
// component's try/catch and silently take the no-WebGL path instead.
vi.mock("@xterm/xterm", () => ({
  Terminal: vi.fn().mockImplementation(function () {
    return termInstance;
  }),
}));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: vi.fn().mockImplementation(function () {
    return { fit: vi.fn() };
  }),
}));
// jsdom has no WebGL2, so the real WebglAddon logs a context-creation error to
// stderr before the component's try/catch swallows it. Stub it as a no-op addon
// (the mocked Terminal.loadAddon never calls activate) so the terminal-I/O tests
// run cleanly through the WebGL-present path.
vi.mock("@xterm/addon-webgl", () => ({
  WebglAddon: vi.fn().mockImplementation(function () {
    return {
      onContextLoss: vi.fn(),
      dispose: vi.fn(),
    };
  }),
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
  termInstance.onKey.mockClear();
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

  // xterm answers the terminal queries it parses through onData. The
  // snapshot's queries were answered when they ran; answering them again on
  // replay typed `ESC[?1;2c` onto the shell's line on every remount.
  it("does not send what xterm answers while replaying the snapshot", async () => {
    termInstance.write.mockImplementationOnce((_d: unknown, cb?: () => void) => {
      const onData = termInstance.onData.mock.calls[0][0] as (d: string) => void;
      onData("\x1b[?1;2c"); // xterm's answer to an `ESC[c` in the history
      cb?.();
    });
    render(<SessionTerminalTab sessionId="s1" active={true} />);
    await waitFor(() => expect(termInstance.write).toHaveBeenCalled());
    const onData = termInstance.onData.mock.calls[0][0] as (d: string) => void;
    onData("\x1b[?1;2c"); // a live query, after the replay, is answered
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("terminal_input", {
        sessionId: "s1",
        data: "\x1b[?1;2c",
      }),
    );
    expect(
      invokeMock.mock.calls.filter(([cmd]) => cmd === "terminal_input"),
    ).toHaveLength(1);
  });

  // A key typed during the replay still reaches the shell (through onKey,
  // which xterm fires only for the keyboard), while the answers stay muted.
  it("sends keys typed during the replay, not xterm's answers", async () => {
    termInstance.write.mockImplementationOnce((_d: unknown, cb?: () => void) => {
      const onData = termInstance.onData.mock.calls[0][0] as (d: string) => void;
      const onKey = termInstance.onKey.mock.calls[0][0] as (e: { key: string }) => void;
      onKey({ key: "l" });
      onData("l"); // xterm's onData for the same key — muted, not doubled
      onData("\x1b[?1;2c");
      cb?.();
    });
    render(<SessionTerminalTab sessionId="s1" active={true} />);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("terminal_input", { sessionId: "s1", data: "l" }),
    );
    const onKey = termInstance.onKey.mock.calls[0][0] as (e: { key: string }) => void;
    onKey({ key: "x" }); // after the replay onData carries keys; onKey sends nothing
    expect(
      invokeMock.mock.calls.filter(([cmd]) => cmd === "terminal_input"),
    ).toEqual([["terminal_input", { sessionId: "s1", data: "l" }]]);
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
