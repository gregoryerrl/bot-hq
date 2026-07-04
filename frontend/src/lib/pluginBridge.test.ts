import { describe, expect, it } from "vitest";
import {
  classifyPluginMessage,
  detectSchemeForm,
  expectedOrigins,
  pluginEntryUrl,
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
