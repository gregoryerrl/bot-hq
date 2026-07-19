import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { SessionView } from "./SessionView";

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

// Keyed invoke mock: `get_session` must return a session row or the view
// renders its not-found state; everything else gets an empty default.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) => {
    switch (cmd) {
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
      screen.getByPlaceholderText("Broadcast to Brian + Rain…"),
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
      screen.getByPlaceholderText("Broadcast to Brian + Rain…"),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Terminal" }));
    expect(panel("Terminal").className).not.toContain("hidden");
    expect(panel("Context").className).toContain("hidden");

    fireEvent.click(screen.getByRole("button", { name: "Workspace" }));
    expect(panel("Workspace").className).not.toContain("hidden");
    expect(panel("Terminal").className).toContain("hidden");
  });
});
