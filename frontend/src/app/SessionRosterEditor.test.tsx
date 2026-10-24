import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { SessionPolicyPanel } from "./SessionPolicyPanel";
import type { ParticipantChangeView, SessionParticipantControls } from "../lib/participantChanges";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mock = vi.mocked(invoke);
const roster = [
  { id: 1, slug: "hands", role_display_name: "Builder", model_display_name: "Old", participation_mode: "active", enabled: true },
  { id: 2, slug: "eyes", role_display_name: "Reviewer", model_display_name: "Old", participation_mode: "active", enabled: true },
];
let controls: SessionParticipantControls;
let rows = roster;
let fail = false;
function mount() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  render(<QueryClientProvider client={client}><SessionPolicyPanel sessionId="s1" open onClose={() => {}} /></QueryClientProvider>);
  return client;
}
function sent() { return mock.mock.calls.filter(([cmd]) => cmd === "change_session_participant"); }
function changeRow(status: ParticipantChangeView["status"]): ParticipantChangeView {
  return { id: "saved", request: { kind: "remove", participant_id: 2 }, status, error: status === "failed" ? "Candidate refused; previous runtime restored" : null };
}
beforeEach(() => {
  rows = roster;
  fail = false;
  controls = { revision: 7, closed: false, max_participants: 8, selections: [{ participant_id: 1, model_id: "old" }, { participant_id: 2, model_id: "old" }], latest_change: null };
  mock.mockReset();
  mock.mockImplementation(async (cmd, raw) => {
    const args = raw as Record<string, unknown>;
    if (cmd === "list_session_participants") return rows;
    if (cmd === "get_session_participant_controls") return { ...controls };
    if (cmd === "get_session_policy") return {};
    if (cmd === "get_session_tool_gate") return [];
    if (cmd === "list_roles") return [{ id: 4, display_name: "Reviewer", default_model_id: "new", archived: false }];
    if (cmd === "list_models") return [{ id: "old", display_name: "Old", runtime: "claude_code" }, { id: "new", display_name: "New", runtime: "codex" }];
    if (cmd === "change_session_participant") {
      if (fail) throw { kind: "Internal", message: "reply lost" };
      controls.latest_change = { ...changeRow("pending"), id: args.requestId as string, request: args.request as ParticipantChangeView["request"] };
      return controls.latest_change;
    }
    if (cmd === "cancel_session_participant_change") { controls.latest_change = changeRow("cancelled"); return true; }
    return null;
  });
});

describe("Session Settings live agent controls", () => {
  it("adds the selected role/model through the panel with revision and stable request identity", async () => {
    mount();
    await screen.findByRole("option", { name: "Reviewer" });
    fireEvent.change(screen.getByLabelText("Role"), { target: { value: "4" } });
    expect(screen.getByLabelText("Model")).toHaveValue("new");
    fireEvent.change(screen.getByLabelText("Label (optional)"), { target: { value: " Auditor " } });
    fireEvent.click(screen.getByRole("button", { name: "Add agent" }));
    await screen.findByText("Agent change waiting for the current turn to finish.");
    expect(sent()).toHaveLength(1);
    expect(sent()[0][1]).toEqual({ sessionId: "s1", revision: 7, requestId: expect.any(String), request: { kind: "add", role_id: 4, model_id: "new", label: "Auditor" } });
    expect(screen.getByRole("button", { name: "Add agent" })).toBeDisabled();
    expect(screen.getByText(/can consume substantial usage/)).toBeVisible();
  });

  it("switches only the chosen participant and cancels the queued request", async () => {
    mount();
    const picker = await screen.findByLabelText("Model for Builder · Old");
    await waitFor(() => expect(picker).toBeEnabled());
    fireEvent.change(picker, { target: { value: "new" } });
    fireEvent.click(within(picker.closest("li")!).getByRole("button", { name: "Change model" }));
    await screen.findByText("Agent change waiting for the current turn to finish.");
    expect(sent()[0][1]).toMatchObject({ request: { kind: "switch_model", participant_id: 1, model_id: "new" } });
    fireEvent.click(screen.getByRole("button", { name: "Cancel pending change" }));
    await screen.findByText("Agent change cancelled.");
    expect(mock).toHaveBeenCalledWith("cancel_session_participant_change", { sessionId: "s1", requestId: (sent()[0][1] as { requestId: string }).requestId });
  });

  it("removes an agent by id; keeps the last active participant", async () => {
    rows = [roster[0], { ...roster[1], participation_mode: "on_mention" }];
    mount();
    const remove = await screen.findByRole("button", { name: "Remove Reviewer · Old" });
    await waitFor(() => expect(remove).toBeEnabled());
    expect(screen.getByRole("button", { name: "Remove Builder · Old" })).toBeDisabled();
    fireEvent.click(remove);
    await waitFor(() => expect(sent()).toHaveLength(1));
    expect(sent()[0][1]).toMatchObject({ request: { kind: "remove", participant_id: 2 } });
  });

  it.each(["applying", "failed", "applied"] as const)("reads a %s change after reopening the panel", async (status) => {
    controls.latest_change = changeRow(status);
    mount();
    const text = { applying: "Applying agent change…", failed: "Agent change failed.", applied: "Agent change applied." }[status];
    expect(await screen.findByText(text)).toBeVisible();
    expect(screen.queryByRole("button", { name: "Cancel pending change" })).toBeNull();
    if (status === "failed") expect(screen.getByText(/Candidate refused/)).toBeVisible();
  });

  it("retries a lost reply with the same ID instead of inviting twice", async () => {
    fail = true;
    mount();
    const remove = await screen.findByRole("button", { name: "Remove Reviewer · Old" });
    await waitFor(() => expect(remove).toBeEnabled());
    fireEvent.click(remove);
    await screen.findByText("reply lost");
    fail = false;
    fireEvent.click(screen.getByRole("button", { name: "Retry same request" }));
    await screen.findByText("Agent change waiting for the current turn to finish.");
    expect(sent()).toHaveLength(2);
    expect(sent()[1][1]).toEqual(sent()[0][1]);
  });

  it("disables closed sessions and the backend's invitation cap", async () => {
    controls.closed = true;
    controls.max_participants = 2;
    mount();
    await screen.findByText("Reopen this session to change its agents.");
    expect(screen.getByRole("button", { name: "Remove Reviewer · Old" })).toBeDisabled();
    expect(screen.getByLabelText("Role")).toBeDisabled();
    expect(screen.getByText("This session has reached its limit of 2 agents.")).toBeVisible();
  });
});
