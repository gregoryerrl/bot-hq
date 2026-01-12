import { describe, it, expect } from "vitest";
import {
  parseAgentConfig,
  serializeAgentConfig,
  DEFAULT_AGENT_CONFIG,
  type AgentConfig,
} from "@/lib/agents/config-types";

describe("parseAgentConfig", () => {
  it("returns default config for null input", () => {
    const result = parseAgentConfig(null);
    expect(result).toEqual(DEFAULT_AGENT_CONFIG);
  });

  it("returns default config for empty string", () => {
    const result = parseAgentConfig("");
    expect(result).toEqual(DEFAULT_AGENT_CONFIG);
  });

  it("returns default config for invalid JSON", () => {
    const result = parseAgentConfig("not valid json");
    expect(result).toEqual(DEFAULT_AGENT_CONFIG);
  });

  it("parses valid JSON config", () => {
    const config: AgentConfig = {
      approvalRules: ["custom rule"],
      blockedCommands: ["bad command"],
      customInstructions: "Do something",
      allowedPaths: ["/path/to/allow"],
    };
    const result = parseAgentConfig(JSON.stringify(config));
    expect(result).toEqual(config);
  });

  it("fills missing fields with defaults", () => {
    const partial = { approvalRules: ["my rule"] };
    const result = parseAgentConfig(JSON.stringify(partial));
    expect(result.approvalRules).toEqual(["my rule"]);
    expect(result.blockedCommands).toEqual(DEFAULT_AGENT_CONFIG.blockedCommands);
    expect(result.customInstructions).toEqual(DEFAULT_AGENT_CONFIG.customInstructions);
    expect(result.allowedPaths).toEqual(DEFAULT_AGENT_CONFIG.allowedPaths);
  });
});

describe("serializeAgentConfig", () => {
  it("serializes config to JSON string", () => {
    const config: AgentConfig = {
      approvalRules: ["rule1"],
      blockedCommands: ["cmd1"],
      customInstructions: "instructions",
      allowedPaths: ["/path"],
    };
    const result = serializeAgentConfig(config);
    expect(JSON.parse(result)).toEqual(config);
  });
});
