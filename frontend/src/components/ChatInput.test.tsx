import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, beforeEach, vi } from "vitest";

// The composer registers a drag-drop listener through the Tauri webview API on
// mount; outside a webview the dynamic import must resolve to a harmless stub
// or every render in this file would reject in the background.
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: async () => () => {},
  }),
}));
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
  it("keeps the box WRITABLE while busy — Stage replaces Send, Pause stays", () => {
    // The Stage toggle (2026-08-15): rc3 D33's rule was about messages
    // LANDING mid-turn, never about composing. While a turn is in flight the
    // textarea stays open, nothing can SEND (no Send button exists), and the
    // submit slot is Stage — a queued delivery at the next turn boundary.
    render(
      <ChatInput
        activity="busy"
        busy={{ hands: true, eyes: false }}
        busyLabel={LABEL}
        onSend={() => {}}
        onStage={() => {}}
        onCancel={() => {}}
      />,
    );
    expect(screen.getByRole("textbox")).toBeEnabled();
    expect(screen.getByRole("button", { name: "Stage" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
    expect(screen.getByRole("button", { name: "Pause" })).toBeInTheDocument();
    // rc3 D10: the busy participant is named ROLE · Model, never by a slug or
    // an agent name.
    expect(screen.getByText("HANDS · Opus")).toBeInTheDocument();
    expect(screen.queryByText("hands")).toBeNull();
    expect(screen.getByText("is working")).toBeInTheDocument();
  });

  it("puts the status and the buttons on ONE footer row under the box, never beside it", () => {
    // Round 11 (the user: "an empty space above Stage and Pause"). The buttons
    // sat beside the auto-growing textarea, bottom-aligned, so every extra
    // line the user typed opened a taller blank column above them, and the
    // locked state spent a third row on the status line. Now: the box, then a
    // footer row that carries the status on the left and the buttons on the
    // right — the same row count as before when locked, no gap at any height.
    render(
      <ChatInput
        activity="busy"
        busy={{ hands: true, eyes: false }}
        busyLabel={LABEL}
        onSend={() => {}}
        onStage={() => {}}
        onCancel={() => {}}
      />,
    );
    const pause = screen.getByRole("button", { name: "Pause" });
    const stage = screen.getByRole("button", { name: "Stage" });
    const status = screen.getByText("is working");
    const row = pause.parentElement!;
    expect(row).toBe(stage.parentElement);
    expect(row.contains(status)).toBe(true);
    // The box is a full-width row of its own ABOVE that footer, so nothing
    // sits beside it to leave a column empty…
    const box = screen.getByRole("textbox").parentElement!;
    expect(row.contains(box)).toBe(false);
    expect(box.nextElementSibling).toBe(row);
    // …and it can shrink with the pane (a textarea's min-content is ~20
    // characters; `min-width:auto` would overflow the split's narrow end).
    expect(box.className).toContain("min-w-0");
    expect(row.className).not.toContain("items-end");
  });

  it("stages the typed message, locks it, and un-stages back to editable", async () => {
    const onStage = vi.fn().mockResolvedValue(undefined);
    const onUnstage = vi.fn().mockResolvedValue(undefined);
    const { rerender } = render(
      <ChatInput
        activity="busy"
        busy={{ hands: true }}
        busyLabel={LABEL}
        onSend={() => {}}
        onStage={onStage}
        onUnstage={onUnstage}
        onCancel={() => {}}
      />,
    );
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "queued while they work" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Stage" }));
    await waitFor(() =>
      expect(onStage).toHaveBeenCalledWith("queued while they work"),
    );

    // The parent flips `staged`: the box locks read-only, the toggle shows
    // Staged ✓, and the text stays visible — it IS the queued message.
    rerender(
      <ChatInput
        activity="busy"
        busy={{ hands: true }}
        busyLabel={LABEL}
        onSend={() => {}}
        onStage={onStage}
        onUnstage={onUnstage}
        onCancel={() => {}}
        staged
        stagedText="queued while they work"
      />,
    );
    const box = screen.getByRole("textbox");
    expect(box).toHaveValue("queued while they work");
    expect(box).toHaveAttribute("readonly");
    fireEvent.click(screen.getByRole("button", { name: "Staged ✓" }));
    await waitFor(() => expect(onUnstage).toHaveBeenCalledTimes(1));
  });

  // issues.md #3, the "sometimes": a delivery that lands while THIS session's
  // view is unmounted (the dashboard, another session) reaches no
  // `session:stage_delivered` handler, so a persisted draft outlives the
  // message it was and refills the box on return. Staging moves the text's home
  // to the backend, so the draft key goes with it — nothing left to resurrect.
  it("drops the persisted draft at stage time and writes it back on unstage", async () => {
    const onStage = vi.fn().mockResolvedValue(undefined);
    const onUnstage = vi.fn().mockResolvedValue(undefined);
    const { rerender } = render(
      <ChatInput
        draftKey="bothq:draft:s-stage"
        activity="busy"
        busy={{ hands: true }}
        busyLabel={LABEL}
        onSend={() => {}}
        onStage={onStage}
        onUnstage={onUnstage}
        onCancel={() => {}}
      />,
    );
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "queued while they work" },
    });
    expect(localStorage.getItem("bothq:draft:s-stage")).toBe("queued while they work");
    fireEvent.click(screen.getByRole("button", { name: "Stage" }));
    await waitFor(() => expect(onStage).toHaveBeenCalledTimes(1));
    // Staged: the backend owns the text now; the key is gone.
    expect(localStorage.getItem("bothq:draft:s-stage")).toBeNull();
    expect(screen.getByRole("textbox")).toHaveValue("queued while they work");

    rerender(
      <ChatInput
        draftKey="bothq:draft:s-stage"
        activity="busy"
        busy={{ hands: true }}
        busyLabel={LABEL}
        onSend={() => {}}
        onStage={onStage}
        onUnstage={onUnstage}
        onCancel={() => {}}
        staged
        stagedText="queued while they work"
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Staged ✓" }));
    await waitFor(() => expect(onUnstage).toHaveBeenCalledTimes(1));
    // Editing again: the text is a draft again, persisted again.
    expect(localStorage.getItem("bothq:draft:s-stage")).toBe("queued while they work");
    localStorage.removeItem("bothq:draft:s-stage");
  });

  it("clears the draft when the staged delivery lands", () => {
    const { rerender } = render(
      <ChatInput
        activity="busy"
        busy={{ hands: true }}
        busyLabel={LABEL}
        onSend={() => {}}
        onStage={() => {}}
        onCancel={() => {}}
        deliveredTick={0}
      />,
    );
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "about to deliver" },
    });
    rerender(
      <ChatInput
        activity="idle"
        busy={{}}
        busyLabel={LABEL}
        onSend={() => {}}
        onStage={() => {}}
        onCancel={() => {}}
        deliveredTick={1}
      />,
    );
    expect(screen.getByRole("textbox")).toHaveValue("");
  });

  // These two pin ChatInput's HALF of the remount case: the box seeds from the
  // persisted draft, so what it comes back holding is decided entirely by
  // whether that key survived. Neither executes the line that removes it —
  // that is `SessionView`'s handler, pinned by
  // `SessionView.test.tsx`'s "clears the persisted draft even with the composer
  // unmounted". An earlier version of this comment claimed the removal was
  // done "by the SessionView handler" while the test did it inline, which is a
  // doc claim about a line the test never runs.
  it("seeds empty when the draft was cleared while it was unmounted", () => {
    // SETUP, not the behaviour under test: stand the world up as it is AFTER
    // SessionView's handler ran and the composer was unmounted for it.
    localStorage.setItem("bothq:draft:s1", "queued while they work");
    localStorage.removeItem("bothq:draft:s1");
    render(
      <ChatInput
        draftKey="bothq:draft:s1"
        activity="idle"
        busy={{}}
        busyLabel={LABEL}
        onSend={() => {}}
        onStage={() => {}}
        onCancel={() => {}}
        deliveredTick={1}
      />,
    );
    expect(screen.getByRole("textbox")).toHaveValue("");
  });

  // The inverse, so the pair states the whole rule: a surviving draft comes
  // back in the box. This is the symptom the user reported, reproduced at the
  // component level.
  it("seeds the delivered text back if the draft survived", () => {
    localStorage.setItem("bothq:draft:s2", "queued while they work");
    render(
      <ChatInput
        draftKey="bothq:draft:s2"
        activity="idle"
        busy={{}}
        busyLabel={LABEL}
        onSend={() => {}}
        onStage={() => {}}
        onCancel={() => {}}
        deliveredTick={1}
      />,
    );
    expect(screen.getByRole("textbox")).toHaveValue("queued while they work");
    localStorage.removeItem("bothq:draft:s2");
  });

  // The composer's half of the "I have to manually clear it" report. The box
  // itself was never the culprit — the delivered message was re-staged behind
  // it (see `SessionView`'s re-stage effect), so `staged`/`stagedText` came
  // back legitimately and this component did the right thing with them.
  //
  // Pinned anyway, because it is the invariant the fix relies on: a delivery
  // clears the box, a LATER genuine stage still rehydrates it, and the two are
  // told apart by the props rather than by timing.
  it("clears on delivery and still rehydrates a genuinely new staged message", () => {
    const { rerender } = render(
      <ChatInput
        activity="busy"
        busy={{ hands: true }}
        busyLabel={LABEL}
        onSend={() => {}}
        onStage={() => {}}
        onCancel={() => {}}
        staged
        stagedText="queued while they work"
        deliveredTick={0}
      />,
    );
    expect(screen.getByRole("textbox")).toHaveValue("queued while they work");

    // Delivery fires. The tick bumps, but the staged query has NOT come back
    // yet — this is the window the bug lived in.
    rerender(
      <ChatInput
        activity="idle"
        busy={{}}
        busyLabel={LABEL}
        onSend={() => {}}
        onStage={() => {}}
        onCancel={() => {}}
        staged
        stagedText="queued while they work"
        deliveredTick={1}
      />,
    );
    expect(screen.getByRole("textbox")).toHaveValue("");

    // The refetch lands: still clear.
    rerender(
      <ChatInput
        activity="idle"
        busy={{}}
        busyLabel={LABEL}
        onSend={() => {}}
        onStage={() => {}}
        onCancel={() => {}}
        staged={false}
        stagedText={null}
        deliveredTick={1}
      />,
    );
    expect(screen.getByRole("textbox")).toHaveValue("");

    // And the guard must not be permanent: a NEW staged message still
    // rehydrates, which is what the effect is actually for (a reload while
    // staged). Suppressing that forever would be a quieter second bug.
    rerender(
      <ChatInput
        activity="busy"
        busy={{ hands: true }}
        busyLabel={LABEL}
        onSend={() => {}}
        onStage={() => {}}
        onCancel={() => {}}
        staged
        stagedText="a different queued message"
        deliveredTick={1}
      />,
    );
    expect(screen.getByRole("textbox")).toHaveValue("a different queued message");
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

  it("cannot SEND while a participant works, even with a question parked", () => {
    // **This test has changed subject twice, and each time the subject was
    // the contract.** At rc3 D33 it locked the whole box ("users are never
    // allowed to type while agents are working"). The Stage toggle
    // (2026-08-15) moved the lock from the BOX to the SEND: composing is
    // free, but no message can LAND while a turn is in flight — the submit
    // slot is Stage (a boundary-queued delivery), and Pause remains the one
    // interrupt.
    render(
      <ChatInput
        activity="awaiting_user"
        busy={{ hands: true, eyes: false }}
        busyLabel={LABEL}
        onSend={() => {}}
        onStage={() => {}}
        onCancel={() => {}}
      />,
    );
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
    expect(screen.getByRole("button", { name: "Stage" })).toBeInTheDocument();
    expect(screen.getByRole("textbox")).toBeEnabled();
    expect(screen.getByText("HANDS · Opus")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Pause" })).toBeInTheDocument();
  });

  it("gates SEND on the busy MAP even when the session enum has lost it", () => {
    // The mechanism behind the test above, isolated: `SessionActivity::derive`
    // ranks `awaiting` above `busy`, so the collapsed enum cannot answer "is
    // anyone working". The per-participant map can, and the backend emits it on
    // every activity event whatever the derived state.
    //
    // Mutation check for whoever edits `isLocked`: drop the `anyBusy(busy)` arm
    // and this is the test that goes red — a Send button under a working ring.
    render(
      <ChatInput
        activity="awaiting_user"
        busy={{ eyes: true }}
        busyLabel={LABEL}
        onSend={() => {}}
        onStage={() => {}}
        onCancel={() => {}}
      />,
    );
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
    expect(screen.getByRole("button", { name: "Stage" })).toBeInTheDocument();
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
    // No Pause (no onCancel), no Send; the box is open for composing and
    // Stage is present but disabled — this surface has no onStage either.
    expect(screen.getByRole("textbox")).toBeEnabled();
    expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
    expect(screen.getByRole("button", { name: "Stage" })).toBeDisabled();
    expect(screen.getByText("HANDS · Opus")).toBeInTheDocument();
  });

  it("reads Stopping… with a disabled Pause and a disabled box while cancelling", () => {
    // Mid-kill is the one state where even composing is off: the box is about
    // to change hands and a keystroke race helps nobody.
    render(
      <ChatInput activity="cancelling" onSend={() => {}} onCancel={() => {}} />,
    );
    expect(screen.getByRole("textbox")).toBeDisabled();
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

// ===========================================================================
// ideas.md 2026-08-24 — the `#` document picker (same walker as `@`)
// ===========================================================================

describe("ChatInput document picker", () => {
  const DOCS = [
    { key: "doc/investigate", label: "doc/investigate (investigate)", insert: "(session doc: investigate)" },
    { key: "bot-hq/conventions.md", label: "bot-hq/conventions.md", insert: "/Users/x/library/projects/bot-hq/conventions.md" },
  ];

  function type(text: string) {
    const box = screen.getByRole("textbox") as HTMLTextAreaElement;
    fireEvent.change(box, {
      target: { value: text, selectionStart: text.length },
    });
    return box;
  }

  it("offers the documents on # and keeps the TOKEN in the box (round 13)", () => {
    render(<ChatInput onSend={() => {}} docMentionables={DOCS} />);
    type("read #doc");
    expect(screen.getAllByRole("option")).toHaveLength(1);
    fireEvent.mouseDown(screen.getAllByRole("option")[0]);
    expect(screen.getByRole("textbox")).toHaveValue("read #doc/investigate ");
  });

  it("a CL file's token is project-namespaced; the abs path leaves only at Send", async () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    render(<ChatInput onSend={onSend} docMentionables={DOCS} />);
    const box = type("#bot");
    fireEvent.keyDown(box, { key: "Enter" });
    expect(box).toHaveValue("#bot-hq/conventions.md ");
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() =>
      expect(onSend).toHaveBeenCalledWith(
        "`/Users/x/library/projects/bot-hq/conventions.md`",
      ),
    );
  });

  it("does not open inside a word (issue#5 is prose, not a mention)", () => {
    render(<ChatInput onSend={() => {}} docMentionables={DOCS} />);
    type("see issue#5");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("stays out of the way with no documents to offer", () => {
    render(<ChatInput onSend={() => {}} />);
    type("#anything");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("keeps the @ and # sources separate", () => {
    // (attached files also ride the # picker — covered by the paste tests.)
    render(
      <ChatInput
        onSend={() => {}}
        mentionables={[{ slug: "hands", label: "HANDS · Opus" }]}
        docMentionables={DOCS}
      />,
    );
    type("@");
    expect(screen.getAllByRole("option")).toHaveLength(1);
    expect(screen.getAllByRole("option")[0]).toHaveTextContent("HANDS");
    type("#");
    expect(screen.getAllByRole("option")).toHaveLength(2);
    expect(screen.getAllByRole("option")[0]).toHaveTextContent("doc/investigate");
  });
});

// ===========================================================================
// ideas.md 2026-08-24 — `/` promptcodes: token → the configured prompt text
// ===========================================================================

describe("ChatInput promptcode picker", () => {
  const CODES = [
    {
      code: "n-verify",
      prompt:
        "Do n rounds of verification (you decide n), different angles.\nIf a round fails, start over from 1/n",
    },
    { code: "ship", prompt: "Run the gates and commit." },
  ];

  function type(text: string) {
    const box = screen.getByRole("textbox") as HTMLTextAreaElement;
    fireEvent.change(box, {
      target: { value: text, selectionStart: text.length },
    });
    return box;
  }

  it("keeps /code in the box and sends the FULL prompt (round 13)", async () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    render(<ChatInput onSend={onSend} promptcodes={CODES} />);
    const box = type("/n-v");
    fireEvent.keyDown(box, { key: "Enter" });
    expect(box).toHaveValue("/n-verify ");
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() =>
      expect(onSend).toHaveBeenCalledWith(
        "\n> Do n rounds of verification (you decide n), different angles.\n> If a round fails, start over from 1/n\n",
      ),
    );
  });

  it("backticks escape: no picker over `/code`, and it sends literally", async () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    render(<ChatInput onSend={onSend} promptcodes={CODES} />);
    const box = type("type `/n-verify");
    expect(screen.queryByRole("listbox")).toBeNull();
    fireEvent.change(box, {
      target: { value: "type `/n-verify` yourself", selectionStart: 25 },
    });
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() =>
      expect(onSend).toHaveBeenCalledWith("type `/n-verify` yourself"),
    );
  });

  it("does not open inside a path (foo/bar is prose)", () => {
    render(<ChatInput onSend={() => {}} promptcodes={CODES} />);
    type("look at foo/bar");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("does not open after ./ or ~/ either (e052ae77 — a path stays a path)", () => {
    render(
      <ChatInput
        onSend={() => {}}
        promptcodes={[{ code: "test", prompt: "EXPANDED" }]}
      />,
    );
    type("run ./te");
    expect(screen.queryByRole("listbox")).toBeNull();
    type("cd ~/te");
    expect(screen.queryByRole("listbox")).toBeNull();
    // After whitespace it is a token and the picker serves it.
    type("say /te");
    expect(screen.getAllByRole("option")).toHaveLength(1);
  });

  it("stays out of the way with no codes configured", () => {
    render(<ChatInput onSend={() => {}} />);
    type("/anything");
    expect(screen.queryByRole("listbox")).toBeNull();
  });
});

describe("ChatInput file paste", () => {
  function pasteInto(box: HTMLElement, uriList: string) {
    fireEvent.paste(box, {
      clipboardData: {
        getData: (t: string) => (t === "text/uri-list" ? uriList : ""),
        items: [],
      },
    });
  }

  it("inserts a Finder-copied file as a short #token, expanding at send", async () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    render(<ChatInput onSend={onSend} />);
    const box = screen.getByRole("textbox") as HTMLTextAreaElement;
    fireEvent.change(box, { target: { value: "", selectionStart: 0 } });
    pasteInto(box, "file:///tmp/a%20b.md\n");
    // The token, not the path (round 13) — basename, whitespace sanitized.
    expect(box).toHaveValue("#a-b.md ");
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() =>
      expect(onSend).toHaveBeenCalledWith("`/tmp/a b.md`"),
    );
  });

  it("dedupes same-named attachments with a numbered token", () => {
    render(<ChatInput onSend={() => {}} />);
    const box = screen.getByRole("textbox") as HTMLTextAreaElement;
    pasteInto(box, "file:///one/report.md\n");
    pasteInto(box, "file:///two/report.md\n");
    expect(box).toHaveValue("#report.md #report-2.md ");
  });

  it("leaves plain-text pastes to the browser default", () => {
    render(<ChatInput onSend={() => {}} />);
    const box = screen.getByRole("textbox") as HTMLTextAreaElement;
    pasteInto(box, "");
    expect(box).toHaveValue("");
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
