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

  it("says an agent is still working while awaiting_user leaves the input open", () => {
    // The reported bug: `awaiting` outranks `busy` in the backend derive, so
    // parking a question unlocks the textarea while the agent keeps running
    // tools — and the per-agent status used to render only in the LOCKED
    // branch, so the user saw an open input and assumed the duo had stopped.
    render(
      <ChatInput
        activity="awaiting_user"
        busy={{ brian: true, rain: false }}
        onSend={() => {}}
        onCancel={() => {}}
      />,
    );
    // Input stays usable — answering the parked question is the whole point.
    expect(screen.getByRole("textbox")).toBeEnabled();
    expect(screen.getByText("Waiting on your answer ·")).toBeInTheDocument();
    expect(screen.getByText("Brian")).toBeInTheDocument();
    expect(screen.getByText("— the turn hasn't ended yet.")).toBeInTheDocument();
  });

  it("shows no still-working notice when nobody is busy", () => {
    const { rerender } = render(
      <ChatInput
        activity="awaiting_user"
        busy={{ brian: false, rain: false }}
        onSend={() => {}}
      />,
    );
    expect(screen.queryByText(/still|turn hasn't ended/)).toBeNull();
    expect(screen.queryByText("Brian")).toBeNull();

    // …and none on a plain idle session either.
    rerender(<ChatInput activity="idle" onSend={() => {}} />);
    expect(screen.queryByText("Brian")).toBeNull();
  });

  it("labels a paused session that is still finishing a tool", () => {
    render(
      <ChatInput
        activity="paused"
        busy={{ brian: true, rain: false }}
        onSend={() => {}}
        onResume={() => {}}
      />,
    );
    expect(screen.getByText("Stopping ·")).toBeInTheDocument();
    expect(
      screen.getByText("— finishing the current tool."),
    ).toBeInTheDocument();
    // Paused keeps its own bar + an open textarea for a steer.
    expect(screen.getByRole("textbox")).toBeEnabled();
    expect(screen.getByRole("button", { name: "Resume" })).toBeInTheDocument();
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

describe("ChatInput paused bar", () => {
  it("shows the paused bar with an OPEN textarea while paused", () => {
    render(
      <ChatInput
        activity="paused"
        onSend={() => {}}
        onCancel={() => {}}
        onResume={() => {}}
        onClose={() => {}}
      />,
    );
    // The steer path: textarea + Send stay available under the paused bar.
    expect(screen.getByRole("textbox")).toBeEnabled();
    expect(screen.getByText(/Paused/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Resume" })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Close session" }),
    ).toBeInTheDocument();
    // The Stop button belongs to the locked states, not paused.
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
  });

  it("calls onResume and latches Resuming… until activity leaves paused", async () => {
    const onResume = vi.fn().mockResolvedValue(undefined);
    const { rerender } = render(
      <ChatInput
        activity="paused"
        onSend={() => {}}
        onResume={onResume}
        onClose={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Resume" }));
    await waitFor(() => expect(onResume).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("button", { name: "Resuming…" })).toBeDisabled();
    // Backend resumes → busy event → the bar goes away.
    rerender(
      <ChatInput
        activity="busy"
        busy={{ brian: true, rain: true }}
        onSend={() => {}}
        onResume={onResume}
        onClose={() => {}}
      />,
    );
    expect(screen.queryByText(/Paused/)).toBeNull();
  });

  it("routes Close session to onClose (the parent's confirm flow)", () => {
    const onClose = vi.fn();
    render(
      <ChatInput
        activity="paused"
        onSend={() => {}}
        onResume={() => {}}
        onClose={onClose}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Close session" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("hides the paused bar (and its buttons) outside paused", () => {
    render(
      <ChatInput
        activity="idle"
        onSend={() => {}}
        onResume={() => {}}
        onClose={() => {}}
      />,
    );
    expect(screen.queryByText(/Paused/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Resume" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Close session" })).toBeNull();
  });
});
