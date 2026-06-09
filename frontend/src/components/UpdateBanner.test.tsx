import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { shouldShowUpdateBanner, UpdateBannerView } from "./UpdateBanner";
import type { UpdateInfo } from "../lib/bindings";

const updateAvailable: UpdateInfo = {
  current_version: "0.1.0",
  latest_version: "0.2.0",
  update_available: true,
  release_url: "https://github.com/gregoryerrl/bot-hq/releases/tag/v0.2.0",
  release_notes: "Fixes",
  published_at: "2026-06-10T00:00:00Z",
};

describe("shouldShowUpdateBanner", () => {
  it("hides when there is no update info yet", () => {
    expect(shouldShowUpdateBanner(undefined, null)).toBe(false);
  });

  it("hides when no update is available", () => {
    expect(
      shouldShowUpdateBanner({ ...updateAvailable, update_available: false }, null),
    ).toBe(false);
  });

  it("shows when an update is available and nothing was dismissed", () => {
    expect(shouldShowUpdateBanner(updateAvailable, null)).toBe(true);
  });

  it("hides when the available version was already dismissed", () => {
    expect(shouldShowUpdateBanner(updateAvailable, "0.2.0")).toBe(false);
  });

  it("shows again when a newer version arrives after an older dismissal", () => {
    expect(shouldShowUpdateBanner(updateAvailable, "0.1.5")).toBe(true);
  });
});

describe("UpdateBannerView", () => {
  it("shows the available version with Download and Dismiss actions", () => {
    render(
      <UpdateBannerView
        info={updateAvailable}
        onDownload={() => {}}
        onDismiss={() => {}}
      />,
    );
    expect(screen.getByText(/0\.2\.0/)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /download/i }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /dismiss/i })).toBeInTheDocument();
  });

  it("calls onDownload when Download is clicked", () => {
    const onDownload = vi.fn();
    render(
      <UpdateBannerView
        info={updateAvailable}
        onDownload={onDownload}
        onDismiss={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /download/i }));
    expect(onDownload).toHaveBeenCalledTimes(1);
  });

  it("calls onDismiss when Dismiss is clicked", () => {
    const onDismiss = vi.fn();
    render(
      <UpdateBannerView
        info={updateAvailable}
        onDownload={() => {}}
        onDismiss={onDismiss}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /dismiss/i }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});
