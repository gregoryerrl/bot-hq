import { render } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { PluginHost, promptEndsWithUnfilledSection } from "./PluginHost";
import type { InstalledPluginView } from "../lib/bindings";

// Capture useTauriEvent registrations so tests can fire backend events at
// the mounted host (keyed by event name; PluginHost registers each once).
const { handlers, postPluginEvent } = vi.hoisted(() => ({
  handlers: new Map<string, (payload: unknown) => void>(),
  postPluginEvent: vi.fn(),
}));
vi.mock("../hooks/useTauriEvent", () => ({
  useTauriEvent: (name: string, handler: (payload: unknown) => void) => {
    handlers.set(name, handler);
  },
}));
vi.mock("../lib/pluginBridge", () => ({
  mountPluginBridge: vi.fn(() => () => {}),
  pluginEntryUrl: vi.fn(() => "about:blank"),
  schemeForm: vi.fn(() => "custom"),
  postPluginEvent,
}));

function hostPlugin(over: Partial<InstalledPluginView>): InstalledPluginView {
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

describe("PluginHost push-event scoping (two-plugin contract)", () => {
  beforeEach(() => {
    handlers.clear();
    postPluginEvent.mockClear();
  });

  it("forwards assets_changed into the iframe ONLY for its own plugin id", () => {
    render(<PluginHost plugin={hostPlugin({})} />);
    const fire = handlers.get("plugin:assets_changed");
    expect(fire).toBeTruthy();

    // Another mounted plugin's event: nothing crosses into this iframe —
    // the PLUGINS.md contract is "a file in YOUR served directory changed".
    fire!({ plugin_id: "other-plugin" });
    expect(postPluginEvent).not.toHaveBeenCalled();

    // Its own event: forwarded as the bhq:event topic.
    fire!({ plugin_id: "dev" });
    expect(postPluginEvent).toHaveBeenCalledTimes(1);
    expect(postPluginEvent.mock.calls[0][1]).toBe("plugin_assets_changed");
  });

  it("gates sessions_changed on the list_sessions grant", () => {
    render(<PluginHost plugin={hostPlugin({})} />);
    handlers.get("session:created")!({ session_id: "s-1" });
    expect(postPluginEvent).not.toHaveBeenCalled();

    render(
      <PluginHost
        plugin={hostPlugin({
          manifest: {
            ...hostPlugin({}).manifest,
            requested_capabilities: ["list_sessions"],
          },
        })}
      />,
    );
    handlers.get("session:created")!({ session_id: "s-1" });
    expect(postPluginEvent).toHaveBeenCalledTimes(1);
    expect(postPluginEvent.mock.calls[0][1]).toBe("sessions_changed");
  });
});

describe("promptEndsWithUnfilledSection", () => {
  it("flags a prompt whose last non-empty line ends with a colon", () => {
    expect(
      promptEndsWithUnfilledSection(
        "Craft a material.\n\nTask (what material to craft):",
      ),
    ).toBe(true);
  });

  it("ignores trailing blank lines and whitespace after the colon line", () => {
    expect(
      promptEndsWithUnfilledSection("Do the thing.\n\nTask:\n\n   \n"),
    ).toBe(true);
  });

  it("passes a prompt with a filled final section", () => {
    expect(
      promptEndsWithUnfilledSection("Task:\nBuild the oak shelf material."),
    ).toBe(false);
  });

  it("passes plain prose without colons", () => {
    expect(promptEndsWithUnfilledSection("Summarize the CL for me.")).toBe(
      false,
    );
  });

  it("treats empty and whitespace-only prompts as fine (Rust rejects them)", () => {
    expect(promptEndsWithUnfilledSection("")).toBe(false);
    expect(promptEndsWithUnfilledSection("  \n \n")).toBe(false);
  });

  it("only looks at the LAST non-empty line — a colon mid-prompt is fine", () => {
    expect(
      promptEndsWithUnfilledSection("Sections:\n- a\n- b\nDo it now."),
    ).toBe(false);
  });
});
