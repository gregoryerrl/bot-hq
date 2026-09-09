import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { ContextManager } from "./ContextManager";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

const PROJECTS = [
  {
    name: "_globals",
    display_name: "Global rules",
    working_repo_path: null,
    description: null,
    cl_path: null,
  },
  {
    name: "alpha",
    display_name: "alpha",
    working_repo_path: "/repos/alpha",
    description: null,
    cl_path: null,
  },
  {
    name: "beta",
    display_name: "beta",
    working_repo_path: null,
    description: null,
    cl_path: null,
  },
];

function mockCommands(overrides: Record<string, unknown> = {}) {
  mockInvoke.mockImplementation(async (cmd: string) => {
    if (cmd in overrides) return overrides[cmd];
    if (cmd === "list_projects") return PROJECTS;
    if (cmd === "cl_retrieval_stats") return null;
    return [];
  });
}

function renderManager() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <ContextManager />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("ContextManager", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("lists projects with _globals last and selects the first by default", async () => {
    mockCommands();
    renderManager();

    // alpha renders twice: its sidebar row + the auto-selected header strip.
    const alpha = await screen.findAllByText("alpha");
    expect(alpha.length).toBeGreaterThan(0);
    // Named projects first; the `_globals` bucket is pinned last (its row
    // renders AFTER alpha's in document order).
    const globalsRow = screen.getByText("Global rules");
    expect(
      alpha[0].compareDocumentPosition(globalsRow) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    // The first ordered project (alpha) is auto-selected — its header strip
    // shows the bound working repo.
    expect(await screen.findByText("/repos/alpha")).toBeInTheDocument();
  });

  it("renders the selected project's measurement card", async () => {
    mockCommands();
    renderManager();

    expect(
      await screen.findByText("No retrievals logged yet"),
    ).toBeInTheDocument();
  });

  it("switches selection to another project", async () => {
    mockCommands();
    renderManager();

    fireEvent.click(await screen.findByText("beta"));
    expect(await screen.findByText("no working repo bound")).toBeInTheDocument();
    expect(screen.queryByText("/repos/alpha")).not.toBeInTheDocument();
  });
});
