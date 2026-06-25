import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, beforeEach, vi } from "vitest";
import { ChatInput } from "./ChatInput";

const DRAFT_KEY = "bothq:draft:s-test1234";

describe("ChatInput draft persistence", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("seeds the textarea from localStorage when draftKey is set", () => {
    localStorage.setItem(DRAFT_KEY, "half-typed thought");
    render(<ChatInput draftKey={DRAFT_KEY} onSend={() => {}} />);
    expect(screen.getByRole("textbox")).toHaveValue("half-typed thought");
  });

  it("writes the draft through to localStorage on change", () => {
    render(<ChatInput draftKey={DRAFT_KEY} onSend={() => {}} />);
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "work in progress" },
    });
    expect(localStorage.getItem(DRAFT_KEY)).toBe("work in progress");
  });

  it("removes the key when the box is emptied", () => {
    localStorage.setItem(DRAFT_KEY, "soon gone");
    render(<ChatInput draftKey={DRAFT_KEY} onSend={() => {}} />);
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "" } });
    expect(localStorage.getItem(DRAFT_KEY)).toBeNull();
  });

  it("clears the draft on successful send", async () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    render(<ChatInput draftKey={DRAFT_KEY} onSend={onSend} />);
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "ship it" },
    });
    fireEvent.submit(screen.getByRole("textbox").closest("form")!);
    await waitFor(() => expect(onSend).toHaveBeenCalledWith("ship it"));
    await waitFor(() => {
      expect(screen.getByRole("textbox")).toHaveValue("");
      expect(localStorage.getItem(DRAFT_KEY)).toBeNull();
    });
  });

  it("keeps the draft when send fails", async () => {
    const onSend = vi.fn().mockRejectedValue(new Error("bridge down"));
    render(<ChatInput draftKey={DRAFT_KEY} onSend={onSend} />);
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "do not lose me" },
    });
    fireEvent.submit(screen.getByRole("textbox").closest("form")!);
    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
    expect(screen.getByRole("textbox")).toHaveValue("do not lose me");
    expect(localStorage.getItem(DRAFT_KEY)).toBe("do not lose me");
  });

  it("stays draft-free without a draftKey", () => {
    render(<ChatInput onSend={() => {}} />);
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "ephemeral" },
    });
    expect(localStorage.length).toBe(0);
  });
});

describe("ChatInput turn-status + Stop", () => {
  it("hides the textarea and shows the turn-status + Stop while busy", () => {
    render(
      <ChatInput
        activity="busy"
        busy={{ brian: true, rain: false }}
        onSend={() => {}}
        onCancel={() => {}}
      />,
    );
    // While the duo works the input is replaced by the status line — no textarea.
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.getByRole("button", { name: "Stop" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
    // Per-agent label: Brian (HANDS) is working.
    expect(screen.getByText("Brian")).toBeInTheDocument();
    expect(screen.getByText("is working")).toBeInTheDocument();
  });

  it("labels Rain reviewing when only Rain is busy", () => {
    render(
      <ChatInput
        activity="busy"
        busy={{ brian: false, rain: true }}
        onSend={() => {}}
        onCancel={() => {}}
      />,
    );
    expect(screen.getByText("Rain")).toBeInTheDocument();
    expect(screen.getByText("is reviewing")).toBeInTheDocument();
    expect(screen.queryByText("Brian")).toBeNull();
  });

  it("shows both agents when a broadcast leaves both busy", () => {
    render(
      <ChatInput
        activity="busy"
        busy={{ brian: true, rain: true }}
        onSend={() => {}}
        onCancel={() => {}}
      />,
    );
    expect(screen.getByText("Brian")).toBeInTheDocument();
    expect(screen.getByText("Rain")).toBeInTheDocument();
  });

  it("keeps the textarea + Send on idle and awaiting-user (the user's turn)", () => {
    const { rerender } = render(
      <ChatInput activity="idle" onSend={() => {}} onCancel={() => {}} />,
    );
    expect(screen.getByRole("textbox")).toBeEnabled();
    expect(screen.getByRole("button", { name: "Send" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();

    rerender(
      <ChatInput activity="awaiting_user" onSend={() => {}} onCancel={() => {}} />,
    );
    expect(screen.getByRole("textbox")).toBeEnabled();
    expect(screen.getByRole("button", { name: "Send" })).toBeInTheDocument();
  });

  it("calls onCancel and shows Cancelling… when Stop is pressed", async () => {
    const onCancel = vi.fn().mockResolvedValue(undefined);
    render(
      <ChatInput
        activity="busy"
        busy={{ brian: true, rain: false }}
        onSend={() => {}}
        onCancel={onCancel}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    await waitFor(() => expect(onCancel).toHaveBeenCalledTimes(1));
    expect(
      screen.getByRole("button", { name: "Cancelling…" }),
    ).toBeInTheDocument();
  });

  it("shows the status with no Stop when busy without onCancel", () => {
    render(
      <ChatInput
        activity="busy"
        busy={{ brian: true, rain: false }}
        onSend={() => {}}
      />,
    );
    // No textarea, no Stop (no onCancel), no Send — just the status line.
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
    expect(screen.getByText("Brian")).toBeInTheDocument();
  });

  it("reads Stopping… with a disabled Stop while cancelling", () => {
    render(
      <ChatInput activity="cancelling" onSend={() => {}} onCancel={() => {}} />,
    );
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.getByText(/Stopping/)).toBeInTheDocument();
    const stop = screen.getByRole("button", { name: "Cancelling…" });
    expect(stop).toBeInTheDocument();
    expect(stop).toBeDisabled();
  });
});
