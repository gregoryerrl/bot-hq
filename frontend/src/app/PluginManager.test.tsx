import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { CspConsentSection, PluginCard } from "./PluginManager";
import type { CspExtraOrigins, InstalledPluginView } from "../lib/bindings";

describe("CspConsentSection", () => {
  it("lists the exact origins per directive, scheme stripped", () => {
    const csp: CspExtraOrigins = {
      "script-src": ["https://cdn.jsdelivr.net", "https://unpkg.com"],
      "style-src": ["https://fonts.googleapis.com"],
      "font-src": ["https://fonts.gstatic.com"],
      "img-src": [],
    };
    render(<CspConsentSection csp={csp} />);
    expect(
      screen.getByText(/Can load and run code from: cdn\.jsdelivr\.net, unpkg\.com/),
    ).toBeTruthy();
    expect(
      screen.getByText(/Can load styles from: fonts\.googleapis\.com/),
    ).toBeTruthy();
    expect(
      screen.getByText(/Can load fonts from: fonts\.gstatic\.com/),
    ).toBeTruthy();
    // Empty directive renders no line.
    expect(screen.queryByText(/Can load images from/)).toBeNull();
  });

  it("tolerates omitted directives (wire format skips empty vecs)", () => {
    // The generated type claims all four keys exist; the wire format omits
    // empty ones. The component must not crash on a partial object.
    const partial = {
      "script-src": ["https://cdn.jsdelivr.net"],
    } as unknown as CspExtraOrigins;
    render(<CspConsentSection csp={partial} />);
    expect(
      screen.getByText(/Can load and run code from: cdn\.jsdelivr\.net/),
    ).toBeTruthy();
  });

  it("renders nothing when every directive is empty", () => {
    const empty = {} as unknown as CspExtraOrigins;
    const { container } = render(<CspConsentSection csp={empty} />);
    expect(container.innerHTML).toBe("");
  });
});

function pluginView(over: Partial<InstalledPluginView>): InstalledPluginView {
  return {
    id: "dev",
    name: "Dev Plugin",
    version: "0.1.0",
    enabled: true,
    status: { kind: "Healthy" },
    manifest: {
      id: "dev",
      name: "Dev Plugin",
      version: "0.1.0",
      entry: "index.html",
      api_version: 1,
      requested_capabilities: [],
      slots: [],
      csp_extra_origins: null,
    },
    dir_path: "/home/me/dev-plugin",
    linked: false,
    manifest_drifted: false,
    installed_at: "2026-07-05T00:00:00Z",
    ...over,
  } as InstalledPluginView;
}

describe("PluginCard linked/drift surface", () => {
  const noop = () => {};

  it("shows the linked chip for linked installs only", () => {
    const { rerender } = render(
      <PluginCard
        plugin={pluginView({ linked: true })}
        onToggle={noop}
        onUninstall={noop}
        onReapprove={noop}
        onReinstall={noop}
        busy={false}
      />,
    );
    expect(screen.getByText("linked")).toBeTruthy();
    rerender(
      <PluginCard
        plugin={pluginView({ linked: false })}
        onToggle={noop}
        onUninstall={noop}
        onReapprove={noop}
        onReinstall={noop}
        busy={false}
      />,
    );
    expect(screen.queryByText("linked")).toBeNull();
  });

  it("surfaces manifest drift with a re-approve action", () => {
    const onReapprove = vi.fn();
    render(
      <PluginCard
        plugin={pluginView({ linked: true, manifest_drifted: true })}
        onToggle={noop}
        onUninstall={noop}
        onReapprove={onReapprove}
        onReinstall={noop}
        busy={false}
      />,
    );
    expect(screen.getByText(/Manifest changed on disk/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /re-approve/i }));
    expect(onReapprove).toHaveBeenCalledOnce();
  });

  it("hides the drift banner when the manifest matches", () => {
    render(
      <PluginCard
        plugin={pluginView({ linked: true, manifest_drifted: false })}
        onToggle={noop}
        onUninstall={noop}
        onReapprove={noop}
        onReinstall={noop}
        busy={false}
      />,
    );
    expect(screen.queryByText(/Manifest changed on disk/)).toBeNull();
  });
});
