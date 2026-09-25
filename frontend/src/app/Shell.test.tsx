import { act, render, screen } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { BuildStamp, Shell, WorkingSessions } from "./Shell";
import { useActivityStore } from "../stores/activity";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

// The wire this file exists to pin (EYES ebf148dd's class): the one-time
// diagnostics ask is only reachable because Shell MOUNTS it — the card has
// its own suite, but deleting the mount line would leave that suite green
// while no user ever saw the question.
describe("Shell — the diagnostics-ask mount", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_telemetry_status") {
        return {
          enabled: false,
          asked: false,
          install_id: null,
          endpoint: "",
          queued_bytes: 0,
        };
      }
      if (cmd === "list_installed_plugins") return [];
      if (cmd === "list_pending_tray") return [];
      if (cmd === "check_for_update") {
        return {
          current_version: "1.0.0",
          latest_version: "1.0.0",
          update_available: false,
          release_url: "",
          release_notes: null,
          published_at: null,
        };
      }
      return null;
    });
  });

  it("an unasked install sees the diagnostics card in the shell chrome", async () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={qc}>
        <MemoryRouter>
          <Shell />
        </MemoryRouter>
      </QueryClientProvider>,
    );
    expect(await screen.findByText("DIAGNOSTICS")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Enable" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "No thanks" })).toBeTruthy();
  });
});

// The footer (the user's ask, 2026-09-25): the version and build commit where
// the old copyright line was, the restart state of the program file, and how
// many sessions have a turn in flight.
describe("Shell — the footer's build stamp and working count", () => {
  const build = (over: Record<string, unknown> = {}) => ({
    version: "1.0.6",
    commit: "9bb7e53",
    profile: "release",
    exe_path: "/Users/u/Projects/bot-hq/target/release/bot-hq",
    exe_built_at: "2026-09-25T02:15:41Z",
    exe_state: "current",
    data_dir: "/Users/u/.bot-hq",
    schema_version: 86,
    ...over,
  });
  const renderStamp = (info: unknown) => {
    mockInvoke.mockReset();
    mockInvoke.mockImplementation(async (cmd: string) =>
      cmd === "app_build_info" ? info : null,
    );
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    return render(
      <QueryClientProvider client={qc}>
        <BuildStamp />
      </QueryClientProvider>,
    );
  };

  it("shows the version and the commit the binary was built from", async () => {
    renderStamp(build());
    const stamp = await screen.findByText(/v1\.0\.6/);
    expect(screen.getByTestId("build-stamp")).toHaveTextContent("bot-hq v1.0.6 · 9bb7e53");
    expect(stamp.closest("[data-testid=build-stamp]")?.getAttribute("title")).toContain(
      "Database: migration 86",
    );
    expect(screen.queryByTestId("restart-pending")).toBeNull();
    expect(screen.queryByText(/INDUSTRIAL ORCHESTRATION/)).toBeNull();
  });

  it("marks a debug build dev, and a release with no stamp shows the version only", async () => {
    const { unmount } = renderStamp(build({ commit: null, profile: "debug" }));
    await screen.findByText(/v1\.0\.6/);
    expect(screen.getByTestId("build-stamp")).toHaveTextContent("bot-hq v1.0.6 · dev");
    unmount();
    renderStamp(build({ commit: null }));
    await screen.findByText(/v1\.0\.6/);
    expect(screen.getByTestId("build-stamp").textContent).toBe("bot-hq v1.0.6");
  });

  it("says restart pending when the program file changed, and rebuild when it is gone", async () => {
    const { unmount } = renderStamp(build({ exe_state: "changed" }));
    expect(await screen.findByTestId("restart-pending")).toHaveTextContent("restart pending");
    unmount();
    renderStamp(build({ exe_state: "missing" }));
    expect(await screen.findByTestId("rebuild-needed")).toHaveTextContent("rebuild needed");
    expect(screen.queryByTestId("restart-pending")).toBeNull();
  });

  it("counts sessions with a turn in flight, a paused one still finishing included", () => {
    useActivityStore.setState({ bySession: {}, busyBySession: {} });
    const { rerender } = render(<WorkingSessions />);
    expect(screen.queryByTestId("working-sessions")).toBeNull();
    act(() =>
      useActivityStore.setState({
        bySession: { a: "busy", b: "idle", c: "paused", d: "cancelling" },
        busyBySession: { a: { hands: true }, b: {}, c: { eyes: true }, d: {} },
      }),
    );
    rerender(<WorkingSessions />);
    expect(screen.getByTestId("working-sessions")).toHaveTextContent("3 working");
    act(() =>
      useActivityStore.setState({ bySession: { b: "idle" }, busyBySession: { b: {} } }),
    );
    rerender(<WorkingSessions />);
    expect(screen.getByTestId("working-sessions")).toHaveTextContent("0 working");
  });
});

// The wire: the stamp is only seen because Shell MOUNTS it in the footer.
describe("Shell — the footer mounts the build stamp", () => {
  it("renders the version where the copyright line was", async () => {
    mockInvoke.mockReset();
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "app_build_info") {
        return {
          version: "1.0.6",
          commit: "9bb7e53",
          profile: "release",
          exe_path: null,
          exe_built_at: null,
          exe_state: "changed",
          data_dir: "/d",
          schema_version: 86,
        };
      }
      if (cmd === "get_telemetry_status") {
        return { enabled: false, asked: true, install_id: null, endpoint: "", queued_bytes: 0 };
      }
      if (cmd === "list_installed_plugins" || cmd === "list_pending_tray") return [];
      return null;
    });
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={qc}>
        <MemoryRouter>
          <Shell />
        </MemoryRouter>
      </QueryClientProvider>,
    );
    expect(await screen.findByTestId("restart-pending")).toBeTruthy();
    expect(screen.getByTestId("build-stamp")).toHaveTextContent("bot-hq v1.0.6 · 9bb7e53");
  });
});
