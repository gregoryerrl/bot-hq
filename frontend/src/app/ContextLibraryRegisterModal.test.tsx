import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { RegisterProjectModal } from "./ContextLibraryRegisterModal";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

describe("RegisterProjectModal", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("creates a managed project via cl_create_project (no folder indexing)", async () => {
    mockInvoke.mockResolvedValue(undefined);
    const onRegistered = vi.fn();
    const onClose = vi.fn();
    render(
      <RegisterProjectModal open onClose={onClose} onRegistered={onRegistered} />,
    );

    fireEvent.change(screen.getByPlaceholderText("project-name"), {
      target: { value: "widget" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^create$/i }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("cl_create_project", {
        name: "widget",
        workingRepoPath: null,
        description: null,
      }),
    );
    // The default flow must never index a folder as CL content.
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "cl_register_project",
      expect.anything(),
    );
    expect(mockInvoke).not.toHaveBeenCalledWith("cl_rescan", expect.anything());
    expect(onRegistered).toHaveBeenCalledWith("widget");
    expect(onClose).toHaveBeenCalled();
  });

  it("binds a working repo without indexing it", async () => {
    mockInvoke.mockResolvedValue(undefined);
    render(
      <RegisterProjectModal open onClose={() => {}} onRegistered={() => {}} />,
    );
    fireEvent.change(screen.getByPlaceholderText("project-name"), {
      target: { value: "widget" },
    });
    fireEvent.change(
      screen.getByPlaceholderText(
        "(where sessions run — leave blank for none)",
      ),
      { target: { value: "/Users/me/Projects/widget" } },
    );
    fireEvent.click(screen.getByRole("button", { name: /^create$/i }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("cl_create_project", {
        name: "widget",
        workingRepoPath: "/Users/me/Projects/widget",
        description: null,
      }),
    );
  });

  it("advanced cl_path indexes an existing folder via cl_register_project + rescan", async () => {
    mockInvoke.mockResolvedValue(undefined);
    render(
      <RegisterProjectModal open onClose={() => {}} onRegistered={() => {}} />,
    );

    fireEvent.change(screen.getByPlaceholderText("project-name"), {
      target: { value: "docs" },
    });
    fireEvent.click(screen.getByRole("button", { name: /advanced/i }));
    fireEvent.change(
      screen.getByPlaceholderText(
        "(advanced — for a docs folder, not a code repo)",
      ),
      { target: { value: "/Users/me/docs" } },
    );
    fireEvent.click(screen.getByRole("button", { name: /index folder/i }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("cl_register_project", {
        name: "docs",
        displayName: "docs",
        workingRepoPath: null,
        clPath: "/Users/me/docs",
        description: null,
      }),
    );
    expect(mockInvoke).toHaveBeenCalledWith("cl_rescan", { project: "docs" });
  });

  it("renders nothing when closed", () => {
    const { container } = render(
      <RegisterProjectModal
        open={false}
        onClose={() => {}}
        onRegistered={() => {}}
      />,
    );
    expect(container).toBeEmptyDOMElement();
  });
});
