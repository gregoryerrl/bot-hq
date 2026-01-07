#!/usr/bin/env node

/**
 * Approval Gate Hook
 *
 * Blocks commands that would publish code to GitHub.
 * Agents work freely on local branches - publishing happens
 * through Bot-HQ's draft PR approval flow instead.
 */

// Commands that are always blocked (would publish code)
const BLOCKED_PATTERNS = [
  "git push",
  "git push ",
  "gh pr create",
  "gh pr merge",
  "npm publish",
  "yarn publish",
  "pnpm publish",
];

async function main() {
  let inputData = "";
  for await (const chunk of process.stdin) {
    inputData += chunk;
  }

  try {
    const input = JSON.parse(inputData);
    const { tool_name, tool_input } = input;

    // Only check Bash commands
    if (tool_name !== "Bash" || !tool_input?.command) {
      output({ decision: "allow" });
      return;
    }

    const command = tool_input.command.toLowerCase();

    // Block publishing commands
    for (const pattern of BLOCKED_PATTERNS) {
      if (command.includes(pattern.toLowerCase())) {
        output({
          decision: "deny",
          reason: `Publishing commands are blocked. Complete your work and Bot-HQ will handle the PR creation.`,
        });
        return;
      }
    }

    // Allow everything else
    output({ decision: "allow" });
  } catch (err) {
    output({ decision: "deny", reason: `Hook error: ${err.message}` });
  }
}

function output(result) {
  console.log(JSON.stringify(result));
}

main();
