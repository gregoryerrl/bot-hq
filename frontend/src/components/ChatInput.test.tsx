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

/**
 * The slug -> display resolver SessionView hands down (rc3 D10). Slugs are
 * internal; what the line prints is `ROLE · Model`.
 */
const LABEL = (slug: string) =>
  ({ hands: "HANDS · Opus", eyes: "EYES · Sonnet" })[slug] ?? slug;

describe("ChatInput turn-status + Stop", () => {
  it("hides the textarea and shows the turn-status + Pause while busy", () => {
    render(
      <ChatInput
        activity="busy"
        busy={{ hands: true, eyes: false }}
        busyLabel={LABEL}
        onSend={() => {}}
        onCancel={() => {}}
      />,
    );
    // While a turn is in flight the input is replaced by the status line.
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.getByRole("button", { name: "Pause" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
    // rc3 D10: the busy participant is named ROLE · Model, never by a slug or
    // an agent name.
    expect(screen.getByText("HANDS · Opus")).toBeInTheDocument();
    expect(screen.queryByText("hands")).toBeNull();
    expect(screen.getByText("is working")).toBeInTheDocument();
  });

  it("names the busy participant by role and model whichever slot it is in", () => {
    // The old line hardcoded slot 1 = "Rain" + the verb "is reviewing", which
    // is bot-hq claiming to know what a role MEANS. It knows only that a turn
    // is in flight, and who the roster says that participant is.
    render(
      <ChatInput
        activity="busy"
        busy={{ hands: false, eyes: true }}
        busyLabel={LABEL}
        onSend={() => {}}
        onCancel={() => {}}
      />,
    );
    expect(screen.getByText("EYES · Sonnet")).toBeInTheDocument();
    expect(screen.getByText("is working")).toBeInTheDocument();
    expect(screen.queryByText("HANDS · Opus")).toBeNull();
  });

  it("shows every participant a broadcast left busy", () => {
    render(
      <ChatInput
        activity="busy"
        busy={{ hands: true, eyes: true }}
        busyLabel={LABEL}
        onSend={() => {}}
        onCancel={() => {}}
      />,
    );
    expect(screen.getByText("HANDS · Opus")).toBeInTheDocument();
    expect(screen.getByText("EYES · Sonnet")).toBeInTheDocument();
  });

  it("falls back to the slug when the roster has no row for a busy author", () => {
    // A participant that left the session still has to be attributable — an
    // unattributed "is working" is worse than an internal key.
    render(
      <ChatInput
        activity="busy"
        busy={{ ghost: true }}
        busyLabel={LABEL}
        onSend={() => {}}
        onCancel={() => {}}
      />,
    );
    expect(screen.getByText("ghost")).toBeInTheDocument();
  });

  it("keeps the textarea + Send on idle and awaiting-user (the user's turn)", () => {
    const { rerender } = render(
      <ChatInput activity="idle" onSend={() => {}} onCancel={() => {}} />,
    );
    expect(screen.getByRole("textbox")).toBeEnabled();
    expect(screen.getByRole("button", { name: "Send" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();

    rerender(
      <ChatInput activity="awaiting_user" onSend={() => {}} onCancel={() => {}} />,
    );
    expect(screen.getByRole("textbox")).toBeEnabled();
    expect(screen.getByRole("button", { name: "Send" })).toBeInTheDocument();
  });

  it("LOCKS while a participant works, even with a question parked", () => {
    // **This test changed subject at rc3 D33, and the old subject was the bug.**
    //
    // It used to assert that `awaiting_user` + busy left the textarea OPEN, on
    // the reasoning that answering a parked question is the whole point. The
    // user's screenshot is what that reasoning produces in practice: an open
    // box, a banner claiming a halt, and the status line underneath correctly
    // naming a participant mid-turn.
    //
    // The rule is now the user's: *"users are never allowed to type while
    // agents are working."* A parked question is answered in the tray — a
    // click, not a sentence — so unlocking the box was never needed to answer
    // it. And if the user genuinely wants to speak, Pause is right there.
    render(
      <ChatInput
        activity="awaiting_user"
        busy={{ hands: true, eyes: false }}
        busyLabel={LABEL}
        onSend={() => {}}
        onCancel={() => {}}
      />,
    );
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.getByText("HANDS · Opus")).toBeInTheDocument();
    // The one way back to the box.
    expect(screen.getByRole("button", { name: "Pause" })).toBeInTheDocument();
  });

  it("locks on the busy MAP even when the session enum has lost it", () => {
    // The mechanism behind the test above, isolated: `SessionActivity::derive`
    // ranks `awaiting` above `busy`, so the collapsed enum cannot answer "is
    // anyone working". The per-participant map can, and the backend emits it on
    // every activity event whatever the derived state.
    //
    // Mutation check for whoever edits `isLocked`: drop the `anyBusy(busy)` arm
    // and this is the test that goes red.
    render(
      <ChatInput
        activity="awaiting_user"
        busy={{ eyes: true }}
        busyLabel={LABEL}
        onSend={() => {}}
        onCancel={() => {}}
      />,
    );
    expect(screen.queryByRole("textbox")).toBeNull();
  });

  it("unlocks the moment the last participant stops, with no halt required", () => {
    // "No halt = no type" is a floor, not a ceiling: the box opens when nobody
    // is working, whether the session halted, went idle, or simply finished a
    // lap. Nothing has to grant permission.
    render(
      <ChatInput
        activity="awaiting_user"
        busy={{ hands: false, eyes: false }}
        busyLabel={LABEL}
        onSend={() => {}}
        onCancel={() => {}}
      />,
    );
    expect(screen.getByRole("textbox")).toBeEnabled();
  });

  it("shows no still-working notice when nobody is busy", () => {
    const { rerender } = render(
      <ChatInput
        activity="awaiting_user"
        busy={{ hands: false, eyes: false }}
        busyLabel={LABEL}
        onSend={() => {}}
      />,
    );
    expect(screen.queryByText(/still|turn hasn't ended/)).toBeNull();
    expect(screen.queryByText("HANDS · Opus")).toBeNull();

    // …and none on a plain idle session either.
    rerender(<ChatInput activity="idle" onSend={() => {}} />);
    expect(screen.queryByText("HANDS · Opus")).toBeNull();
  });

  it("labels a paused session that is still finishing a tool", () => {
    render(
      <ChatInput
        activity="paused"
        busy={{ hands: true, eyes: false }}
        busyLabel={LABEL}
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

  it("calls onCancel and shows Pausing… when Pause is pressed", async () => {
    const onCancel = vi.fn().mockResolvedValue(undefined);
    render(
      <ChatInput
        activity="busy"
        busy={{ hands: true, eyes: false }}
        busyLabel={LABEL}
        onSend={() => {}}
        onCancel={onCancel}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    await waitFor(() => expect(onCancel).toHaveBeenCalledTimes(1));
    expect(
      screen.getByRole("button", { name: "Pausing…" }),
    ).toBeInTheDocument();
  });

  it("shows the status with no Pause when busy without onCancel", () => {
    render(
      <ChatInput
        activity="busy"
        busy={{ hands: true, eyes: false }}
        busyLabel={LABEL}
        onSend={() => {}}
      />,
    );
    // No textarea, no Stop (no onCancel), no Send — just the status line.
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
    expect(screen.getByText("HANDS · Opus")).toBeInTheDocument();
  });

  it("reads Stopping… with a disabled Pause while cancelling", () => {
    render(
      <ChatInput activity="cancelling" onSend={() => {}} onCancel={() => {}} />,
    );
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.getByText(/Stopping/)).toBeInTheDocument();
    const stop = screen.getByRole("button", { name: "Pausing…" });
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
    expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
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
        busy={{ hands: true, eyes: true }}
        busyLabel={LABEL}
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

// ===========================================================================
// rc3 D17 — the `@` picker
// ===========================================================================

describe("ChatInput mention picker", () => {
  const ROSTER = [
    { slug: "hands", label: "HANDS · Claude Opus 5" },
    { slug: "eyes", label: "EYES · DeepSeek V4 Pro" },
    { slug: "advisor", label: "ADVISOR · Claude Opus 5" },
  ];

  beforeEach(() => localStorage.clear());

  /** Type `text` with the caret left at the end, which is where a picker looks. */
  function type(text: string) {
    const box = screen.getByRole("textbox") as HTMLTextAreaElement;
    fireEvent.change(box, {
      target: { value: text, selectionStart: text.length },
    });
    return box;
  }

  it("offers this session's participants, and only those", () => {
    render(<ChatInput onSend={() => {}} mentionables={ROSTER} />);
    type("@");
    const options = screen.getAllByRole("option").map((o) => o.textContent);
    expect(options).toHaveLength(3);
    expect(options.join(" ")).toContain("ADVISOR");
    // Mentioning a non-participant is not an error to report — it is a thing
    // the UI cannot express, which is the point of a picker over free text.
    type("@nobody");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("filters as you type, by slug or by what is displayed", () => {
    render(<ChatInput onSend={() => {}} mentionables={ROSTER} />);
    type("@adv");
    expect(screen.getAllByRole("option")).toHaveLength(1);
    // The user reads "ADVISOR", not "advisor" — matching the label is what
    // makes the picker findable by the name that is on screen.
    type("@DeepSeek");
    expect(screen.getAllByRole("option")[0]).toHaveTextContent("EYES");
  });

  it("inserts the slug the backend parses, not the label", () => {
    render(<ChatInput onSend={() => {}} mentionables={ROSTER} />);
    type("look @adv");
    fireEvent.mouseDown(screen.getAllByRole("option")[0]);
    // `@advisor ` — the trailing space is deliberate, so the next word is not
    // swallowed into the slug.
    expect(screen.getByRole("textbox")).toHaveValue("look @advisor ");
  });

  it("does not open on an email address", () => {
    render(<ChatInput onSend={() => {}} mentionables={ROSTER} />);
    type("mail me at someone@ha");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("Enter picks the highlighted participant instead of sending", () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    render(<ChatInput onSend={onSend} mentionables={ROSTER} />);
    const box = type("@e");
    fireEvent.keyDown(box, { key: "Enter" });
    expect(onSend).not.toHaveBeenCalled();
    expect(box).toHaveValue("@eyes ");
    // …and with the picker closed, Enter sends as usual.
    fireEvent.keyDown(box, { key: "Enter" });
    expect(onSend).toHaveBeenCalledWith("@eyes");
  });

  it("arrow keys move the highlight", () => {
    render(<ChatInput onSend={() => {}} mentionables={ROSTER} />);
    const box = type("@");
    fireEvent.keyDown(box, { key: "ArrowDown" });
    fireEvent.keyDown(box, { key: "Enter" });
    expect(box).toHaveValue("@eyes ");
  });

  it("Escape dismisses the picker without dismissing what was typed", () => {
    render(<ChatInput onSend={() => {}} mentionables={ROSTER} />);
    const box = type("@adv");
    fireEvent.keyDown(box, { key: "Escape" });
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(box).toHaveValue("@adv");
  });

  it("stays out of the way when there is no roster to offer", () => {
    render(<ChatInput onSend={() => {}} />);
    type("@anything");
    expect(screen.queryByRole("listbox")).toBeNull();
  });
});

describe("ChatInput staged answers (rc3 D34)", () => {
  it("enables Send on an empty box when picks are staged", async () => {
    // Answering without commentary is a complete response — the user's tinker
    // flow in reverse: sometimes the picks ARE the whole reply.
    const onSend = vi.fn().mockResolvedValue(undefined);
    render(<ChatInput onSend={onSend} stagedAnswers={2} />);
    expect(screen.getByText("+2 answers")).toBeInTheDocument();
    const send = screen.getByRole("button", { name: "Send" });
    expect(send).toBeEnabled();
    fireEvent.click(send);
    await waitFor(() => expect(onSend).toHaveBeenCalledWith(""));
  });

  it("keeps Send disabled on an empty box with nothing staged", () => {
    render(<ChatInput onSend={() => {}} stagedAnswers={0} />);
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    expect(screen.queryByText(/answers?$/)).toBeNull();
  });
});
