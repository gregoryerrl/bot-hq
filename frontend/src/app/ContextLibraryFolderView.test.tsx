import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { FolderView } from "./ContextLibraryFolderView";
import { invoke } from "@tauri-apps/api/core";
import type { ClFolderView, ProjectView } from "../lib/bindings";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

const folder = (
  project_id: string,
  folder_path: string,
  description: string,
): ClFolderView => ({
  id: 1,
  project_id,
  folder_path,
  description,
  tags: null,
  created_at: "",
  updated_at: "",
});

const proj = (over: Partial<ProjectView>): ProjectView => ({
  name: "p",
  display_name: "p",
  working_repo_path: null,
  description: null,
  cl_path: null,
  ...over,
});

describe("FolderView", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("edits and saves a folder description via cl_set_folder_description", async () => {
    mockInvoke.mockResolvedValue(undefined);
    const onSaved = vi.fn();
    render(
      <FolderView
        tab={{ kind: "folder", project: "p", folderPath: "archive" }}
        folders={[folder("p", "archive", "old stuff")]}
        project={null}
        onSaved={onSaved}
        onProjectChanged={() => {}}
      />,
    );

    const desc = screen.getByPlaceholderText(/what this folder holds/i);
    expect(desc).toHaveValue("old stuff");
    expect(screen.getByRole("button", { name: /save folder/i })).toBeDisabled();

    fireEvent.change(desc, { target: { value: "archived audits" } });
    const save = screen.getByRole("button", { name: /save folder/i });
    expect(save).toBeEnabled();

    fireEvent.click(save);
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("cl_set_folder_description", {
        project: "p",
        folderPath: "archive",
        description: "archived audits",
        tags: null,
      }),
    );
    expect(onSaved).toHaveBeenCalled();
  });

  it("labels the project-root folder with the project name", () => {
    render(
      <FolderView
        tab={{ kind: "folder", project: "bot-hq", folderPath: "" }}
        folders={[]}
        project={null}
        onSaved={() => {}}
        onProjectChanged={() => {}}
      />,
    );
    expect(screen.getByText("bot-hq")).toBeInTheDocument();
    expect(screen.getByText("project root")).toBeInTheDocument();
  });

  it("shows the registration section on a project root and unbinds the repo", async () => {
    mockInvoke.mockResolvedValue(undefined);
    const onProjectChanged = vi.fn();
    render(
      <FolderView
        tab={{ kind: "folder", project: "bot-hq", folderPath: "" }}
        folders={[]}
        project={proj({
          name: "bot-hq",
          display_name: "bot-hq",
          working_repo_path: "/Users/me/Projects/bot-hq",
          cl_path: "/Users/me/.bot-hq/projects/bot-hq",
        })}
        onSaved={() => {}}
        onProjectChanged={onProjectChanged}
      />,
    );

    expect(
      screen.getByText("/Users/me/.bot-hq/projects/bot-hq"),
    ).toBeInTheDocument();
    expect(
      screen.getByDisplayValue("/Users/me/Projects/bot-hq"),
    ).toBeInTheDocument();

    // Unbind is confirm-gated: open the dialog, then confirm.
    fireEvent.click(
      screen.getByRole("button", { name: /unbind working repo/i }),
    );
    fireEvent.click(screen.getByRole("button", { name: /^unbind$/i }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("cl_unregister_project", {
        name: "bot-hq",
      }),
    );
    expect(onProjectChanged).toHaveBeenCalled();
  });

  it("hard-deletes a managed project (incl. files) via cl_delete_project", async () => {
    mockInvoke.mockResolvedValue(undefined);
    const onProjectGone = vi.fn();
    render(
      <FolderView
        tab={{ kind: "folder", project: "widget", folderPath: "" }}
        folders={[]}
        project={proj({ name: "widget", display_name: "widget", cl_path: null })}
        onSaved={() => {}}
        onProjectChanged={() => {}}
        onProjectGone={onProjectGone}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /delete project/i }));
    // Managed project → the "also delete files" checkbox is offered.
    fireEvent.click(screen.getByLabelText(/also delete the cl files/i));
    fireEvent.click(screen.getByRole("button", { name: /^delete$/i }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("cl_delete_project", {
        name: "widget",
        deleteClDir: true,
      }),
    );
    expect(onProjectGone).toHaveBeenCalledWith("widget");
  });

  it("renames a project via cl_rename_project", async () => {
    mockInvoke.mockResolvedValue(undefined);
    const onProjectGone = vi.fn();
    render(
      <FolderView
        tab={{ kind: "folder", project: "old", folderPath: "" }}
        folders={[]}
        project={proj({ name: "old", display_name: "old" })}
        onSaved={() => {}}
        onProjectChanged={() => {}}
        onProjectGone={onProjectGone}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /^rename$/i }));
    fireEvent.change(screen.getByDisplayValue("old"), {
      target: { value: "new" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("cl_rename_project", {
        name: "old",
        newName: "new",
      }),
    );
    expect(onProjectGone).toHaveBeenCalledWith("old", "new");
  });
});
