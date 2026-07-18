import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { SessionContextTab } from "./SessionContextTab";

const files = [
  {
    id: 1,
    project_id: "acme-app",
    file_path: "conventions.md",
    description: "project conventions",
    tags: null,
    created_at: "",
    updated_at: "",
  },
  {
    id: 2,
    project_id: "acme-app",
    file_path: "plans/handoff.md",
    description: "a handoff",
    tags: null,
    created_at: "",
    updated_at: "",
  },
];

// Switchable per-test state consumed by the invoke mock below.
const state: {
  project: string | null;
  fileContent: Record<string, unknown>;
} = {
  project: "acme-app",
  fileContent: {},
};

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) => {
    switch (cmd) {
      case "get_session_project_info":
        return Promise.resolve({
          project: state.project,
          provenance: "repo_basename",
        });
      case "cl_index_search":
        return Promise.resolve(state.project ? files : []);
      case "cl_folder_search":
        return Promise.resolve([]);
      case "cl_proposal_counts":
        return Promise.resolve([
          { project_id: "acme-app", open_count: 2 },
        ]);
      case "cl_read_file":
        return Promise.resolve(state.fileContent);
      case "cl_list_proposals":
        return Promise.resolve([]);
      default:
        return Promise.resolve([]);
    }
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

function renderTab() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <SessionContextTab sessionId="s1" />
    </QueryClientProvider>,
  );
}

describe("SessionContextTab", () => {
  it("shows an empty state for repo-less sessions", async () => {
    state.project = null;
    renderTab();
    expect(
      await screen.findByText(/No project is bound to this session/),
    ).toBeInTheDocument();
  });

  it("renders the project tree and opens a file in the lean editor", async () => {
    state.project = "acme-app";
    state.fileContent = {
      project: "acme-app",
      file_path: "conventions.md",
      content: "# Conventions\n",
      size_bytes: 14,
      truncated: false,
      binary: false,
    };
    renderTab();
    // Project name chip + Proposals badge from cl_proposal_counts.
    expect(await screen.findByText("acme-app")).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument();
    // Tree: root file + nested folder file.
    fireEvent.click(await screen.findByRole("button", { name: /conventions\.md/ }));
    // findByDisplayValue waits out the cl_read_file query resolving into the
    // textarea (findByRole would race it and read an empty value). The matcher
    // is the whitespace-normalized form — testing-library trims the element's
    // value before comparing, but not the matcher string.
    const editor = await screen.findByDisplayValue("# Conventions");
    expect(editor).toHaveAccessibleName("Edit conventions.md");
    // Editing enables Save.
    fireEvent.change(editor, { target: { value: "# Conventions\nmore\n" } });
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  });

  it("refuses edits on truncated or binary files", async () => {
    state.project = "acme-app";
    state.fileContent = {
      project: "acme-app",
      file_path: "conventions.md",
      content: "lossy",
      size_bytes: 5,
      truncated: false,
      binary: true,
    };
    renderTab();
    fireEvent.click(await screen.findByRole("button", { name: /conventions\.md/ }));
    expect(await screen.findByText("read-only")).toBeInTheDocument();
    expect(
      screen.getByRole("textbox", { name: "Edit conventions.md" }),
    ).toHaveAttribute("readonly");
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
  });

  it("switches to the Proposals docket", async () => {
    state.project = "acme-app";
    renderTab();
    fireEvent.click(await screen.findByRole("button", { name: /Proposals/ }));
    // ProposalQueue renders (its empty state, since cl_list_proposals → []).
    expect(await screen.findByText(/[Nn]o.*proposals/)).toBeInTheDocument();
  });
});
