import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MeasurementView } from "./MeasurementView";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

function renderView() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MeasurementView project="p" />
    </QueryClientProvider>,
  );
}

describe("MeasurementView", () => {
  beforeEach(() => mockInvoke.mockReset());

  const STATS = {
    event_count: 12,
    distinct_sessions: 4,
    total_tokens: 6000,
    total_atoms: 30,
    stale_hits: 3,
    empty_returns: 2,
    avg_tokens_per_event: 500,
    avg_tokens_per_session: 1500,
    stale_hit_rate: 0.1,
    empty_return_rate: 0.1667,
  };

  it("renders retrieval telemetry from cl_retrieval_stats", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "cl_retrieval_stats") return STATS;
      return null;
    });

    renderView();

    // Await a data-dependent tile so the query has resolved past "Loading…".
    // Locale-independent assertions: the toFixed rates, not comma-grouped ints.
    expect(await screen.findByText("10.0%")).toBeInTheDocument(); // stale-hit rate
    expect(screen.getByText("Tokens / session")).toBeInTheDocument();
    expect(screen.getByText("16.7%")).toBeInTheDocument(); // retrieval-miss rate
  });

  it("shows an empty state when no retrievals are logged", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "cl_retrieval_stats") return { ...STATS, event_count: 0 };
      return null;
    });

    renderView();

    expect(
      await screen.findByText("No retrievals logged yet"),
    ).toBeInTheDocument();
  });
});
