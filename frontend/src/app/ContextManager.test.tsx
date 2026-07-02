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
    if (cmd === "cl_proposal_counts")
      return [{ project_id: "beta", open_count: 3 }];
    if (cmd === "cl_list_proposals") return [];
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

  it("lists projects with open-proposal badges, _globals last", async () => {
    mockCommands();
    renderManager();

    const alpha = await screen.findByText("alpha");
    // beta renders twice: its sidebar row + the auto-selected header strip.
    expect(screen.getAllByText("beta").length).toBeGreaterThan(0);
    // Named projects first; the `_globals` bucket is pinned last (its row
    // renders AFTER alpha's in document order).
    const globalsRow = screen.getByText("Global rules");
    expect(
      alpha.compareDocumentPosition(globalsRow) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    // beta carries its open-proposal count.
    expect(screen.getByTitle("3 open proposals")).toHaveTextContent("3");
  });

  it("auto-selects the first project with open proposals", async () => {
    mockCommands();
    renderManager();

    // beta (3 open) wins the default selection over alpha (listed first, has
    // a repo): the header strip shows beta's no-repo hint, not alpha's path.
    expect(
      await screen.findByText("no working repo bound"),
    ).toBeInTheDocument();
    expect(screen.queryByText("/repos/alpha")).not.toBeInTheDocument();
  });

  it("switches between Proposals and Measurement pills", async () => {
    mockCommands();
    renderManager();

    // Proposals pill is the default — empty docket state renders.
    expect(await screen.findByText("No open proposals")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /measurement/i }));
    expect(
      await screen.findByText("No retrievals logged yet"),
    ).toBeInTheDocument();
  });

  it("selecting a project shows its docket", async () => {
    mockCommands();
    renderManager();

    fireEvent.click(await screen.findByText("alpha"));
    expect(await screen.findByText("/repos/alpha")).toBeInTheDocument();
    expect(await screen.findByText("No open proposals")).toBeInTheDocument();
  });
});
