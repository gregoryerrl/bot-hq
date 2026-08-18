import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { DocumentPane } from "./DocumentPane";
import type { SessionTrayView } from "../lib/bindings";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const mockInvoke = vi.mocked(invoke);

function trayRow(over: Partial<SessionTrayView> = {}): SessionTrayView {
  return {
    id: 1,
    session_id: "s1",
    choice_id: "c-1",
    agent: "brian",
    kind: "choice",
    prompt: "Ship it?",
    options: ["Yes", "No"],
    status: "pending",
    picked_option: null,
    asked_at: "2026-07-29T00:00:00Z",
    answered_at: null,
    supersedes_id: null,
    command_text: null,
    stale: false,
    ...over,
  } as SessionTrayView;
}

/** The session's roster, as `list_session_participants` returns it (rc3 D10). */
const ROSTER = [
  {
    id: 1,
    slug: "hands",
    role_display_name: "HANDS",
    model_display_name: "Claude Opus 5",
    turn_position: 0,
    participation_mode: "active",
    enabled: true,
  },
];

function renderTray(rows: SessionTrayView[], participants: unknown[] = []) {
  mockInvoke.mockImplementation((cmd: string) => {
    if (cmd === "list_session_tray") return Promise.resolve(rows);
    if (cmd === "discard_choice") return Promise.resolve(true);
    if (cmd === "compute_apply_diff")
      return Promise.resolve({ lines: [], note: null });
    if (cmd === "list_session_participants") return Promise.resolve(participants);
    // session_doc_search and friends are list-shaped; the pane filters them.
    return Promise.resolve([]);
  });
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <DocumentPane sessionId="s1" sessionPhase="apply" />
    </QueryClientProvider>,
  );
}

async function openTray(rows: SessionTrayView[], participants: unknown[] = []) {
  renderTray(rows, participants);
  // The Tray tab is phase-independent and starts closed.
  fireEvent.click(await screen.findByRole("tab", { name: /tray/i }));
}

describe("tray card attribution (rc3 D10)", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("names who asked as ROLE · Model, resolved through the session's roster", async () => {
    // Tested as ONE chain: the stored `agent` slug goes through
    // `list_session_participants` and comes out as the card's asked-by line.
    // The tray was named in the D10 sweep list and was still printing
    // `entry.agent` straight from the row.
    await openTray([trayRow({ agent: "hands" })], ROSTER);

    expect(await screen.findByText("Ship it?")).toBeInTheDocument();
    expect(screen.getByText("HANDS · Claude Opus 5")).toBeInTheDocument();
    // The slug is an internal key; it must not reach the card.
    expect(screen.queryByText("hands")).toBeNull();
  });

  it("does not print a legacy agent name when the roster cannot place it", async () => {
    // A card parked before the rekey keeps `agent = 'brian'` forever — rc3 D10
    // kept that history readable on purpose, but not by that name.
    await openTray([trayRow({ agent: "brian" })], ROSTER);
    await screen.findByText("Ship it?");

    expect(screen.getByText("Unknown participant")).toBeInTheDocument();
    expect(screen.queryByText(/^brian$/i)).toBeNull();
  });
});

describe("tray discard", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("confirms before discarding, then calls discard_choice", async () => {
    await openTray([trayRow()]);
    fireEvent.click(
      await screen.findByRole("button", {
        name: /discard this card without answering/i,
      }),
    );

    // Confirm step exists — a mis-click must not destroy a live question.
    expect(await screen.findByText(/discard this card\?/i)).toBeInTheDocument();
    expect(mockInvoke).not.toHaveBeenCalledWith("discard_choice", expect.anything());

    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("discard_choice", {
        choiceId: "c-1",
      }),
    );
  });

  it("never resolves the choice — discarding is not an answer", async () => {
    await openTray([trayRow()]);
    fireEvent.click(
      await screen.findByRole("button", {
        name: /discard this card without answering/i,
      }),
    );
    fireEvent.click(await screen.findByRole("button", { name: "Discard" }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("discard_choice", {
        choiceId: "c-1",
      }),
    );
    // The whole point of the trash button: nothing is sent back to the agent.
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "resolve_choice",
      expect.anything(),
    );
  });

  it("cancelling the confirm leaves the card alone", async () => {
    await openTray([trayRow()]);
    fireEvent.click(
      await screen.findByRole("button", {
        name: /discard this card without answering/i,
      }),
    );
    fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));

    expect(mockInvoke).not.toHaveBeenCalledWith(
      "discard_choice",
      expect.anything(),
    );
    expect(screen.getByText("Ship it?")).toBeInTheDocument();
  });

  it("does not offer a second way to answer an approval", async () => {
    // **Changed subject at rc3 D33.** This used to check the warning shown when
    // discarding an approval from the tray. Approvals are no longer answerable
    // here at all — they take the input slot (`ApprovalGate`), and leaving an
    // Approve button (or a Discard) in the tray as well would rebuild the exact
    // defect the gate exists to fix: two paths into one row, which is what
    // produced "I answered it and a second one appeared".
    //
    // Discard was also the wrong verb for a gate. Something is synchronously
    // blocked on the answer, so the explicit no is **Reject**, which tells the
    // hook. Discarding just walked away from a held-open command.
    await openTray([
      trayRow({
        command_text: "gh pr create --base staging",
        prompt: "Run gated command in this session's repo?",
        options: ["Approve", "Reject"],
      }),
    ]);
    // rc3 D35 dropped the pointer notice too: the gate replaces the input box
    // AND halts the session, so it is unmissable without the tray narrating
    // it. An approval simply is not tray business.
    expect(
      await screen.findByText(/No pending input — you're all caught up/i),
    ).toBeInTheDocument();
    expect(screen.queryByText(/APPROVAL/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Approve" })).toBeNull();
    expect(
      screen.queryByRole("button", {
        name: /discard this card without answering/i,
      }),
    ).toBeNull();
  });

  it("does not render a HALT as a tray item — it is a declared state", async () => {
    // The reported defect, verbatim: "An agent-declared halt displays 'One
    // item on tray'. There is nothing on tray... halt is not on tray anymore,
    // its a declared state." The halt lives in the banner above the input box;
    // the durable row is an implementation detail the tray must not count.
    await openTray([
      trayRow({
        kind: "halt",
        options: [],
        prompt: "Waiting on the tinker output.",
      }),
    ]);
    expect(
      await screen.findByText(/No pending input — you're all caught up/i),
    ).toBeInTheDocument();
    // Neither a card…
    expect(screen.queryByText(/Waiting on the tinker output/)).toBeNull();
    // …nor a badge: the Tray pill shows no count for it.
    expect(screen.queryByText("1")).toBeNull();
  });

  it("still lists an ordinary question, which IS parkable", async () => {
    // The division D33 draws: a question waits for you and the session carries
    // on, so it keeps its card, its options and its Discard. Only approvals —
    // where something is blocked right now — lose the tray.
    await openTray([
      trayRow({
        command_text: null,
        prompt: "Which branch should this target?",
        options: ["main", "staging"],
      }),
    ]);
    expect(
      await screen.findByText("Which branch should this target?"),
    ).toBeInTheDocument();
    expect(screen.queryByText(/APPROVAL/)).toBeNull();
    expect(
      screen.getByRole("button", {
        name: /discard this card without answering/i,
      }),
    ).toBeInTheDocument();
  });
});

describe("tray staging (rc3 D34)", () => {
  beforeEach(async () => {
    mockInvoke.mockReset();
    // Both stores are module singletons; start each case from a known state.
    const { useActivityStore } = await import("../stores/activity");
    const { useTrayStaging } = await import("../stores/trayStaging");
    useActivityStore.getState().clearSession("s1");
    useTrayStaging.setState({ staged: {} });
  });

  it("STAGES a pick while the box is open, instead of resolving it", async () => {
    // The user's design, verbatim: "remove the send button on tray items. On
    // Halt, sending a message will also send all of the answers on all tray
    // items." No activity event seeded = nobody working = the box is open —
    // the same isLocked rule the composer reads, so the two surfaces agree.
    await openTray([trayRow()]);
    fireEvent.click(await screen.findByRole("button", { name: "Yes" }));

    expect(await screen.findByText(/staged: Yes/)).toBeInTheDocument();
    expect(screen.getByText(/sends with your message/)).toBeInTheDocument();
    // The defect this replaces: the first click fired resolve_choice, which
    // released the ring and locked the box before the other questions could
    // be answered or a message added.
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "resolve_choice",
      expect.anything(),
    );
  });

  it("lets the user change or withdraw a staged pick", async () => {
    await openTray([trayRow()]);
    fireEvent.click(await screen.findByRole("button", { name: "Yes" }));
    // Re-picking overwrites — staged is a draft, not an answer.
    fireEvent.click(screen.getByRole("button", { name: "No" }));
    expect(await screen.findByText(/staged: No/)).toBeInTheDocument();
    expect(screen.queryByText(/staged: Yes/)).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "undo" }));
    expect(screen.queryByText(/staged:/)).toBeNull();
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "resolve_choice",
      expect.anything(),
    );
  });

  it("stages even while participants are working — one batch, no exceptions", async () => {
    // **Changed subject at rc3 D35, hours after it was written.** The first
    // version resolved immediately mid-work, on the theory that a parked
    // question should stay answerable any time. The user hit exactly that
    // branch and overruled it: "Clicking on choices on parked questions
    // immediately sends the answer, I thought I was clear on this that answers
    // will be sent in one batch." A click stages, whatever the session is
    // doing; the composer's Send is the only delivery.
    const { useActivityStore } = await import("../stores/activity");
    useActivityStore.getState().setActivity("s1", "busy", { hands: true });

    await openTray([trayRow()]);
    fireEvent.click(await screen.findByRole("button", { name: "Yes" }));

    expect(await screen.findByText(/staged: Yes/)).toBeInTheDocument();
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "resolve_choice",
      expect.anything(),
    );
  });

  it("stages the Other box as it is typed — there is no send on a question", async () => {
    // "I type on the 'other:' box on question, then click send from the input
    // box, all answers including my message will get sent." Typing stages;
    // emptying withdraws; no button exists to press.
    await openTray([trayRow()]);
    const other = await screen.findByPlaceholderText(/Other — type a custom/);
    fireEvent.change(other, { target: { value: "ship it tomorrow" } });
    expect(await screen.findByText(/staged: ship it tomorrow/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Send$/ })).toBeNull();

    fireEvent.change(other, { target: { value: "" } });
    expect(screen.queryByText(/staged:/)).toBeNull();
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "resolve_choice",
      expect.anything(),
    );
  });
});

// Round 11 (the user's ideas.md): an untagged session doc is a CUSTOM
// document — its own tab beside Tray | I P A V, named by its slug — so a
// session can surface a task checklist, an issue write-up, a scratchpad,
// whatever the work needs, without bending it into an IPAV phase. The phase
// docs' archived versions (`<slug>@<n>`) are untagged too and must NOT become
// tabs.
describe("custom document tabs", () => {
  beforeEach(() => mockInvoke.mockReset());

  const docs = [
    {
      id: 1,
      session_id: "s1",
      slug: "apply",
      body: "# apply doc",
      phase: "apply",
      created_at: "2026-08-18T10:00:00Z",
      updated_at: "2026-08-18T10:00:00Z",
    },
    {
      id: 2,
      session_id: "s1",
      slug: "tasks.md",
      body: "- [ ] first task",
      phase: null,
      created_at: "2026-08-18T10:01:00Z",
      updated_at: "2026-08-18T10:01:00Z",
    },
    {
      id: 3,
      session_id: "s1",
      slug: "plan@1",
      body: "an archived plan",
      phase: null,
      created_at: "2026-08-18T10:02:00Z",
      updated_at: "2026-08-18T10:02:00Z",
    },
  ];

  function renderDocs() {
    mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "session_doc_search") {
        const phase = (args as { phase?: string } | undefined)?.phase;
        return Promise.resolve(
          phase ? docs.filter((d) => d.phase === phase) : docs,
        );
      }
      if (cmd === "compute_apply_diff")
        return Promise.resolve({ lines: [], note: null });
      return Promise.resolve([]);
    });
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    return render(
      <QueryClientProvider client={qc}>
        <DocumentPane sessionId="s1" sessionPhase="apply" />
      </QueryClientProvider>,
    );
  }

  it("offers an untagged doc as its own tab, and not the archived versions", async () => {
    renderDocs();
    // The custom tab appears once the unfiltered search lands.
    const tab = await screen.findByRole("tab", { name: "tasks.md" });
    expect(tab).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "plan@1" })).toBeNull();
    // Phase docs are still where they were — under their phase, not as tabs.
    expect(screen.queryByRole("tab", { name: "apply" })).toBeNull();
  });

  it("shows the custom doc when its tab is picked, and returns to a phase on click", async () => {
    renderDocs();
    fireEvent.click(await screen.findByRole("tab", { name: "tasks.md" }));
    expect(await screen.findByText("first task")).toBeInTheDocument();
    // Back to the phase view via its pill.
    fireEvent.click(screen.getByRole("tab", { name: "A" }));
    await waitFor(() =>
      expect(screen.queryByText("first task")).toBeNull(),
    );
    expect(await screen.findByText("apply doc")).toBeInTheDocument();
  });
});
