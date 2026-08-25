import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { Shell } from "./Shell";

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
