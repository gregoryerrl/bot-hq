#!/usr/bin/env node

const PORT = process.env.PORT || "7890";
const BOT_HQ_URL = process.env.BOT_HQ_URL || `http://localhost:${PORT}`;
const POLL_INTERVAL = 2000;
const TIMEOUT = 300000; // 5 minutes

async function main() {
  let inputData = "";
  for await (const chunk of process.stdin) {
    inputData += chunk;
  }

  try {
    const input = JSON.parse(inputData);
    const { tool_name, tool_input, cwd } = input;

    if (tool_name !== "Bash" || !tool_input?.command) {
      output({ decision: "allow" });
      return;
    }

    const command = tool_input.command;

    // Fetch workspace config
    const cfgRes = await fetch(`${BOT_HQ_URL}/api/workspaces/by-path?path=${encodeURIComponent(cwd)}`);
    if (!cfgRes.ok) {
      output({ decision: "allow" });
      return;
    }

    const workspace = await cfgRes.json();
    const config = workspace.config;

    // Check blocked
    if (matchesAny(command, config.blockedCommands)) {
      output({ decision: "deny", reason: "Command is blocked" });
      return;
    }

    // Check approval required
    if (matchesAny(command, config.approvalRules)) {
      const approval = await createApproval(workspace.id, command);
      const status = await pollApproval(approval.id);
      output({ decision: status === "approved" ? "allow" : "deny" });
      return;
    }

    output({ decision: "allow" });
  } catch (err) {
    output({ decision: "deny", reason: `Hook error: ${err.message}` });
  }
}

function matchesAny(command, rules) {
  const cmd = command.toLowerCase();
  return rules.some(r => cmd.includes(r.toLowerCase()));
}

async function createApproval(workspaceId, command) {
  const res = await fetch(`${BOT_HQ_URL}/api/approvals`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      workspaceId,
      type: "external_command",
      command,
      reason: `Agent requested: ${command}`,
    }),
  });
  return res.json();
}

async function pollApproval(id) {
  const start = Date.now();
  while (Date.now() - start < TIMEOUT) {
    const res = await fetch(`${BOT_HQ_URL}/api/approvals/${id}`);
    if (res.ok) {
      const approval = await res.json();
      if (approval.status !== "pending") return approval.status;
    }
    await new Promise(r => setTimeout(r, POLL_INTERVAL));
  }
  return "rejected";
}

function output(result) {
  console.log(JSON.stringify(result));
}

main();
