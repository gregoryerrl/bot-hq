import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { DiagnosticsAskCard } from "./DiagnosticsAskCard";
import { shouldShowDiagnosticsAsk } from "../lib/telemetry";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

const status = (over: Record<string, unknown> = {}) => ({
  enabled: false,
  asked: false,
  install_id: null,
  endpoint: "",
  queued_bytes: 0,
  ...over,
});

function renderCard() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <DiagnosticsAskCard />
    </QueryClientProvider>,
  );
}

describe("DiagnosticsAskCard", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("shows exactly until asked", () => {
    expect(shouldShowDiagnosticsAsk(status())).toBe(true);
    expect(shouldShowDiagnosticsAsk(status({ asked: true }))).toBe(false);
    expect(shouldShowDiagnosticsAsk(undefined)).toBe(false);
  });

  it("renders for an unasked install and Enable opts in then marks asked", async () => {
    mockInvoke.mockImplementation(async (cmd: string) =>
      cmd === "get_telemetry_status" ? status() : null,
    );
    renderCard();
    const enable = await screen.findByRole("button", { name: "Enable" });
    fireEvent.click(enable);
    await waitFor(() => {
      const calls = mockInvoke.mock.calls.map((c) => c[0]);
      expect(calls).toContain("set_telemetry_enabled");
      expect(calls).toContain("mark_telemetry_asked");
    });
    expect(
      mockInvoke.mock.calls.find((c) => c[0] === "set_telemetry_enabled")?.[1],
    ).toEqual({ enabled: true });
  });

  it("No thanks only marks asked — never enables", async () => {
    mockInvoke.mockImplementation(async (cmd: string) =>
      cmd === "get_telemetry_status" ? status() : null,
    );
    renderCard();
    fireEvent.click(await screen.findByRole("button", { name: "No thanks" }));
    await waitFor(() => {
      expect(mockInvoke.mock.calls.map((c) => c[0])).toContain(
        "mark_telemetry_asked",
      );
    });
    expect(mockInvoke.mock.calls.map((c) => c[0])).not.toContain(
      "set_telemetry_enabled",
    );
  });

  it("stays hidden once asked", async () => {
    mockInvoke.mockImplementation(async (cmd: string) =>
      cmd === "get_telemetry_status" ? status({ asked: true }) : null,
    );
    renderCard();
    await waitFor(() => expect(mockInvoke).toHaveBeenCalled());
    expect(screen.queryByText(/DIAGNOSTICS/)).toBeNull();
  });
});
