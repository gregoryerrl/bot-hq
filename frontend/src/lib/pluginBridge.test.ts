import { describe, expect, it, vi } from "vitest";
import {
  classifyPluginMessage,
  detectSchemeForm,
  expectedOrigins,
  parseSpawnRequest,
  pluginEntryUrl,
  postPluginEvent,
  routeSpawnInvoke,
} from "./pluginBridge";

describe("detectSchemeForm", () => {
  it("recognizes the unix verbatim form", () => {
    expect(detectSchemeForm("bhq-plugin://localhost/probe")).toBe("unix");
  });
  it("treats the https fold as windows", () => {
    expect(detectSchemeForm("https://bhq-plugin.localhost/probe")).toBe("windows");
    expect(detectSchemeForm("http://bhq-plugin.localhost/probe")).toBe("windows");
  });
});

describe("pluginEntryUrl", () => {
  it("puts the id in the host on unix", () => {
    expect(pluginEntryUrl("unix", "deck", "index.html", "n1")).toBe(
      "bhq-plugin://deck/index.html?bhq=n1",
    );
  });
  it("puts the id in the path on windows", () => {
    expect(pluginEntryUrl("windows", "deck", "index.html", "n1")).toBe(
      "https://bhq-plugin.localhost/deck/index.html?bhq=n1",
    );
  });
  it("strips leading slashes from the entry", () => {
    expect(pluginEntryUrl("unix", "deck", "/nested/app.html", "n1")).toBe(
      "bhq-plugin://deck/nested/app.html?bhq=n1",
    );
  });
});

describe("expectedOrigins", () => {
  it("accepts the per-plugin origin and the opaque fallback", () => {
    expect(expectedOrigins("unix", "deck")).toEqual(["bhq-plugin://deck", "null"]);
    expect(expectedOrigins("windows", "deck")).toEqual([
      "https://bhq-plugin.localhost",
      "null",
    ]);
  });
});

describe("classifyPluginMessage", () => {
  const expected = { origins: ["bhq-plugin://deck", "null"], nonce: "n1" };
  const invoke = { type: "bhq:invoke", id: "r1", cmd: "list_sessions", nonce: "n1" };

  it("accepts a well-formed invoke from the right source+origin+nonce", () => {
    const v = classifyPluginMessage(invoke, "bhq-plugin://deck", true, expected);
    expect(v.kind).toBe("invoke");
    if (v.kind === "invoke") expect(v.msg.cmd).toBe("list_sessions");
  });

  it("accepts the opaque-origin fallback", () => {
    expect(classifyPluginMessage(invoke, "null", true, expected).kind).toBe("invoke");
  });

  it("accepts pongs", () => {
    const v = classifyPluginMessage(
      { type: "bhq:pong", nonce: "n1" },
      "bhq-plugin://deck",
      true,
      expected,
    );
    expect(v.kind).toBe("pong");
  });

  it("rejects a wrong source window regardless of payload", () => {
    expect(classifyPluginMessage(invoke, "bhq-plugin://deck", false, expected).kind).toBe(
      "reject",
    );
  });

  it("rejects unexpected origins", () => {
    expect(
      classifyPluginMessage(invoke, "https://evil.example", true, expected).kind,
    ).toBe("reject");
    expect(
      classifyPluginMessage(invoke, "bhq-plugin://other", true, expected).kind,
    ).toBe("reject");
  });

  it("rejects nonce mismatches and missing nonces", () => {
    expect(
      classifyPluginMessage({ ...invoke, nonce: "stolen" }, "null", true, expected).kind,
    ).toBe("reject");
    const { nonce: _nonce, ...noNonce } = invoke;
    expect(classifyPluginMessage(noNonce, "null", true, expected).kind).toBe("reject");
  });

  it("rejects malformed shapes", () => {
    expect(classifyPluginMessage("string", "null", true, expected).kind).toBe("reject");
    expect(classifyPluginMessage(null, "null", true, expected).kind).toBe("reject");
    expect(
      classifyPluginMessage({ type: "bhq:invoke", nonce: "n1" }, "null", true, expected)
        .kind,
    ).toBe("reject");
    expect(
      classifyPluginMessage(
        { type: "bhq:invoke", id: 7, cmd: "x", nonce: "n1" },
        "null",
        true,
        expected,
      ).kind,
    ).toBe("reject");
  });
});

describe("routeSpawnInvoke", () => {
  const spawnArgs = { prompt: "craft materials", project: "cognotify" };

  it("forwards every non-spawn command untouched", () => {
    expect(routeSpawnInvoke("list_sessions", {}, undefined)).toEqual({
      action: "forward",
    });
    expect(
      routeSpawnInvoke("plugin_kv_set", { key: "k", value: "v" }, { granted: true }),
    ).toEqual({ action: "forward" });
  });

  it("fails CLOSED when the mount has no confirm channel", () => {
    const v = routeSpawnInvoke("spawn_session", spawnArgs, undefined);
    expect(v.action).toBe("reject");
  });

  it("forwards without a dialog when the shell view says ungranted", () => {
    // Rust's canonical grant rejection is the single error source; an
    // ungranted plugin never raises a confirm dialog.
    expect(routeSpawnInvoke("spawn_session", spawnArgs, { granted: false })).toEqual({
      action: "forward",
    });
  });

  it("routes granted spawns to the confirm dialog with parsed args", () => {
    const v = routeSpawnInvoke("spawn_session", spawnArgs, { granted: true });
    expect(v.action).toBe("confirm");
    if (v.action === "confirm") {
      expect(v.req).toEqual({
        prompt: "craft materials",
        project: "cognotify",
        title: undefined,
      });
    }
  });
});

describe("postPluginEvent", () => {
  it("posts the exact bhq:event shape the SDK dispatches on", () => {
    const postMessage = vi.fn();
    const iframe = {
      contentWindow: { postMessage },
    } as unknown as HTMLIFrameElement;
    postPluginEvent(iframe, "plugin_assets_changed");
    expect(postMessage).toHaveBeenCalledWith(
      { type: "bhq:event", topic: "plugin_assets_changed" },
      "*",
    );
  });

  it("tolerates a torn-down iframe (no contentWindow)", () => {
    const iframe = { contentWindow: null } as unknown as HTMLIFrameElement;
    expect(() => postPluginEvent(iframe, "sessions_changed")).not.toThrow();
  });
});

describe("parseSpawnRequest", () => {
  it("extracts the request fields and drops non-strings", () => {
    expect(
      parseSpawnRequest({ prompt: "p", project: 42, title: "t", extra: true }),
    ).toEqual({ prompt: "p", project: undefined, title: "t" });
  });

  it("tolerates junk args (Rust re-validates for real)", () => {
    expect(parseSpawnRequest(null)).toEqual({
      prompt: "",
      project: undefined,
      title: undefined,
    });
    expect(parseSpawnRequest("nonsense")).toEqual({
      prompt: "",
      project: undefined,
      title: undefined,
    });
  });
});
