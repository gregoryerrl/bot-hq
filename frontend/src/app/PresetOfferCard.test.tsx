import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { PresetOfferCard, PresetOfferBanner } from "./PresetOfferCard";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

function mockBackend(settings: Record<string, string | null>) {
  mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === "get_app_setting") {
      const key = (args as { key?: string } | undefined)?.key ?? "";
      return settings[key] ?? null;
    }
    return null;
  });
}

function renderWith(ui: React.ReactElement) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>{ui}</MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
});

describe("PresetOfferCard", () => {
  it("renders only on the literal 'pending' (absent or resolved = silence)", async () => {
    // The 0072 contract: an upgrading install has an ABSENT key and must
    // never see the offer.
    mockBackend({ gate_preset_offer: null });
    const { unmount } = renderWith(<PresetOfferCard kind="gates" />);
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "get_app_setting",
        expect.objectContaining({ key: "gate_preset_offer" }),
      ),
    );
    expect(screen.queryByText(/Install basic gates/)).toBeNull();
    unmount();

    mockBackend({ gate_preset_offer: "declined" });
    const second = renderWith(<PresetOfferCard kind="gates" />);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalled());
    expect(screen.queryByText(/Install basic gates/)).toBeNull();
    second.unmount();

    mockBackend({ gate_preset_offer: "pending" });
    renderWith(<PresetOfferCard kind="gates" />);
    expect(await screen.findByText(/Install basic gates/)).toBeTruthy();
  });

  it("install resolves through the kind's own command", async () => {
    mockBackend({ policy_preset_offer: "pending" });
    renderWith(<PresetOfferCard kind="policy" />);
    fireEvent.click(await screen.findByText(/Install basic policy/));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "resolve_policy_preset_offer",
        expect.objectContaining({ install: true }),
      ),
    );
  });
});

describe("PresetOfferBanner", () => {
  it("keys off the offer flags, and dismiss is local-only", async () => {
    // F5: the banner must render for an install WITH sessions — it reads the
    // flags and nothing else (no session query is even made).
    mockBackend({ gate_preset_offer: "pending", policy_preset_offer: null });
    renderWith(<PresetOfferBanner />);
    expect(await screen.findByText(/Starter safety defaults/)).toBeTruthy();

    fireEvent.click(screen.getByLabelText("Dismiss"));
    expect(screen.queryByText(/Starter safety defaults/)).toBeNull();
    // Local dismiss never resolves the offer — no resolve command fired.
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "resolve_gate_preset_offer",
      expect.anything(),
    );
    expect(localStorage.getItem("preset_offer_banner_dismissed")).toBe("1");
  });

  it("stays silent once both offers are resolved", async () => {
    mockBackend({
      gate_preset_offer: "installed",
      policy_preset_offer: "declined",
    });
    renderWith(<PresetOfferBanner />);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalled());
    expect(screen.queryByText(/Starter safety defaults/)).toBeNull();
  });
});
