export interface AgentConfig {
  approvalRules: string[];
  blockedCommands: string[];
  customInstructions: string;
  allowedPaths: string[];
}

export const DEFAULT_AGENT_CONFIG: AgentConfig = {
  approvalRules: ["git push", "git force-push", "npm publish", "yarn publish"],
  blockedCommands: ["rm -rf /", "sudo rm"],
  customInstructions: "",
  allowedPaths: [],
};

export function parseAgentConfig(jsonString: string | null): AgentConfig {
  if (!jsonString) {
    return { ...DEFAULT_AGENT_CONFIG };
  }
  try {
    const parsed = JSON.parse(jsonString);
    return {
      approvalRules: parsed.approvalRules ?? DEFAULT_AGENT_CONFIG.approvalRules,
      blockedCommands: parsed.blockedCommands ?? DEFAULT_AGENT_CONFIG.blockedCommands,
      customInstructions: parsed.customInstructions ?? DEFAULT_AGENT_CONFIG.customInstructions,
      allowedPaths: parsed.allowedPaths ?? DEFAULT_AGENT_CONFIG.allowedPaths,
    };
  } catch {
    return { ...DEFAULT_AGENT_CONFIG };
  }
}

export function serializeAgentConfig(config: AgentConfig): string {
  return JSON.stringify(config);
}

export function commandMatchesRule(command: string, rules: string[]): boolean {
  const normalizedCommand = command.toLowerCase().trim();
  return rules.some(rule => normalizedCommand.includes(rule.toLowerCase()));
}
