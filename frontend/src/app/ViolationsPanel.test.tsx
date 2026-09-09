import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ViolationsPanel } from "./ViolationsPanel";
import { invoke } from "@tauri-apps/api/core";
import type { ViolationRecord } from "../lib/bindings";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

function record(over: Partial<ViolationRecord> = {}): ViolationRecord {
  return {
    ts: "2026-08-12T00:00:00Z",
    session_id: "s1",
    agent: "hands",
    kind: "push_gate",
    action: "git push origin main",
    outcome: "approved",
    detail: null,
    ...over,
  };
}

/** The roster for session `s1`; every other session has none. */
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

function mockBackend(rows: ViolationRecord[], rosterFor = "s1") {
  mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === "read_violations") return rows;
    if (cmd === "list_session_participants") {
      const { sessionId } = (args ?? {}) as { sessionId?: string };
      return sessionId === rosterFor ? ROSTER : [];
    }
    return null;
  });
}

function renderPanel() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <ViolationsPanel />
    </QueryClientProvider>,
  );
}

describe("ViolationsPanel attribution (rc3 D10)", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("names the logged participant as ROLE · Model, resolved through its session's roster", async () => {
    // Tested as ONE chain: the logged `agent` slug goes through
    // `list_session_participants` for that row's session and comes out as the
    // rendered cell. The column used to print `record.agent` under a CSS
    // `capitalize`, which dressed the stored slug up as a display name.
    mockBackend([record()]);
    renderPanel();

    expect(await screen.findByText("HANDS · Claude Opus 5")).toBeInTheDocument();
    expect(screen.queryByText("hands")).toBeNull();
  });

  it("heads the column Participant, not Agent", async () => {
    mockBackend([record()]);
    renderPanel();
    await screen.findByText("HANDS · Claude Opus 5");

    expect(
      screen.getByRole("columnheader", { name: /participant/i }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("columnheader", { name: /^agent$/i })).toBeNull();
  });

  it("does not print a legacy agent name for a row no roster can place", async () => {
    // The log is append-only and never backfilled, so rows written before the
    // rekey keep `agent = 'brian'` / `'rain'` forever (rc3 D10 — "brian and
    // rain's history can be legacy data"). They stay in the table; they are
    // just not named after an agent.
    mockBackend([record({ session_id: "s-old", agent: "brian" })]);
    renderPanel();

    const cell = await screen.findByText("Unknown participant");
    expect(screen.queryByText(/brian/i)).toBeNull();
    // The audit itself still reads — the row is attributed-unknown, not lost.
    // ("Push gate" is also a filter option, so assert it on the ROW.)
    const row = cell.closest("tr")!;
    expect(row.textContent).toMatch(/git push origin main/);
    expect(row.textContent).toMatch(/Push gate/);
  });

  it("resolves each row against ITS OWN session, not the first one it saw", async () => {
    // Rows from different sessions land in one table. Resolving them all
    // through a single roster would mislabel every row from another session.
    mockBackend([
      record({ session_id: "s1", agent: "hands" }),
      record({ session_id: "s-old", agent: "hands", action: "git push --force" }),
    ]);
    renderPanel();
    await screen.findByText("HANDS · Claude Opus 5");

    // Same slug, two sessions: only the one whose roster has it is named.
    expect(screen.getAllByText("HANDS · Claude Opus 5")).toHaveLength(1);
    expect(screen.getByText("Unknown participant")).toBeInTheDocument();
  });
});
