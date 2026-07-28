import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { FileViewerDialog, fileArgInCommand } from "./FileViewerDialog";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const mockInvoke = vi.mocked(invoke);

function file(over: Record<string, unknown> = {}) {
  return {
    path: "/tmp/body.md",
    name: "body.md",
    extension: "md",
    text: "# Issue title\n\nSome body copy.",
    base64: null,
    bytes: 31,
    ...over,
  };
}

describe("fileArgInCommand", () => {
  it("finds the forms agents actually use for gate bodies", () => {
    expect(
      fileArgInCommand('gh issue create --title "x" --body-file /tmp/body.md'),
    ).toBe("/tmp/body.md");
    expect(fileArgInCommand("gh issue comment 426 --body-file /tmp/c.md")).toBe(
      "/tmp/c.md",
    );
    expect(fileArgInCommand("git commit -F /tmp/msg.txt")).toBe("/tmp/msg.txt");
    expect(fileArgInCommand("some --file='/tmp/q uoted.md' thing")).toBe(
      "/tmp/q uoted.md",
    );
  });

  it("finds a bare markdown or image argument", () => {
    expect(fileArgInCommand("gh issue create temp.md")).toBe("temp.md");
    expect(fileArgInCommand("open shots/evidence.png")).toBe(
      "shots/evidence.png",
    );
  });

  it("returns null when no file is referenced", () => {
    expect(fileArgInCommand("git push origin main")).toBeNull();
    expect(fileArgInCommand("cargo test")).toBeNull();
  });
});

describe("FileViewerDialog", () => {
  beforeEach(() => {
    // NOT mockReset(): with a reset mock, an implementation that throws escapes
    // as an uncaught runner error even though the component catches it. Clear
    // the call history and re-stub a benign default instead.
    mockInvoke.mockClear();
    mockInvoke.mockImplementation(() => Promise.resolve(null));
  });

  it("renders nothing when closed", () => {
    const { container } = render(
      <FileViewerDialog target={null} onClose={() => {}} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("reads the file for the given session and renders markdown", async () => {
    mockInvoke.mockResolvedValue(file());
    render(
      <FileViewerDialog
        target={{ sessionId: "s1", path: "/tmp/body.md" }}
        onClose={() => {}}
      />,
    );
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("read_workspace_file", {
        sessionId: "s1",
        path: "/tmp/body.md",
      }),
    );
    // Markdown renderer, not a raw pre — heading becomes a heading.
    expect(
      await screen.findByRole("heading", { name: "Issue title" }),
    ).toBeInTheDocument();
  });

  it("renders an image as a data URL rather than text", async () => {
    mockInvoke.mockResolvedValue(
      file({
        name: "shot.png",
        extension: "png",
        text: null,
        base64: "AAAA",
        path: "/tmp/shot.png",
      }),
    );
    render(
      <FileViewerDialog
        target={{ sessionId: "s1", path: "/tmp/shot.png" }}
        onClose={() => {}}
      />,
    );
    const img = (await screen.findByAltText("shot.png")) as HTMLImageElement;
    expect(img.src).toBe("data:image/png;base64,AAAA");
  });

  it("surfaces a containment refusal instead of failing silently", async () => {
    // Throw synchronously rather than returning a rejected promise: `await`
    // catches both identically, and this leaves no free-floating promise for
    // the runner to flag as unhandled.
    mockInvoke.mockImplementation(() => {
      throw new Error("refused: /etc/passwd is outside this session's workspace");
    });
    render(
      <FileViewerDialog
        target={{ sessionId: "s1", path: "/etc/passwd" }}
        onClose={() => {}}
      />,
    );
    // Assert on the alert role, not the label text — the banner's copy uses a
    // typographic apostrophe (U+2019), which a straight-quote regex misses.
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("outside this session's workspace");
  });

  it("shows inline text without reading any file", async () => {
    render(
      <FileViewerDialog
        target={null}
        inlineTitle="Gated command"
        inlineText="gh issue create --body-file /tmp/x.md"
        onClose={() => {}}
      />,
    );
    expect(
      screen.getByText("gh issue create --body-file /tmp/x.md"),
    ).toBeInTheDocument();
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("closes on the Close button and on Escape", async () => {
    const onClose = vi.fn();
    render(
      <FileViewerDialog target={null} inlineText="x" onClose={onClose} />,
    );
    fireEvent.click(screen.getByRole("button", { name: /close viewer/i }));
    expect(onClose).toHaveBeenCalled();

    onClose.mockClear();
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });
});
