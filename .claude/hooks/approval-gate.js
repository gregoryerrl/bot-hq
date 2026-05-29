#!/usr/bin/env node

/**
 * Approval Gate Hook (PreToolUse:Bash)
 *
 * Blocks commands that would publish code to GitHub / package registries.
 * Agents work freely on local branches; publishing happens through Bot-HQ's
 * draft-PR approval flow instead.
 *
 * IMPORTANT: blocks via EXIT CODE 2 (a claude-code "blocking error"), NOT a
 * JSON {"decision":"deny"} result. Under `--dangerously-skip-permissions`
 * (how HANDS/Emma run) the JSON permission-decision form is SILENTLY IGNORED
 * — bypass skips the permission layer. Exit 2 fires before that layer and is
 * honored; its stderr is fed back to the agent. (Verified 2026-05-29; the
 * prior {decision:deny} version was a no-op for the bypass-mode trio agents.)
 *
 * Fail-open: any parse/read error exits 0 so a hook bug can't brick every
 * Bash call. The policy-driven `policy-check tool-blocklist` hook (injected at
 * spawn from policy.yaml) is the broader, per-project gate; this one is a
 * fixed publish-protection backstop for the bot-hq repo + interactive sessions.
 */

const BLOCKED_PATTERNS = [
  "git push",
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

  let command;
  try {
    const input = JSON.parse(inputData);
    if (input.tool_name !== "Bash" || !input.tool_input?.command) {
      process.exit(0); // not a Bash command → allow
    }
    command = String(input.tool_input.command).trim().toLowerCase();
  } catch {
    process.exit(0); // fail-open: unparseable payload must not brick Bash
  }

  // Prefix-match the whole command — same semantics as the Rust gate's
  // is_blocked_command (`cmd.starts_with(pattern)`). Blocks real invocations
  // (`git push ...`, `gh pr create ...`) with ZERO false positives on mentions
  // (`echo "git push"`, `grep "gh pr create" log`). It does NOT catch a pattern
  // buried mid-compound (`cd x && git push`) — the same accepted gap the Rust
  // gate has; for `git push` the git pre-push hook is the real backstop.
  for (const pattern of BLOCKED_PATTERNS) {
    if (command.startsWith(pattern)) {
      console.error(
        `BLOCKED: publishing commands are gated (matched "${pattern}"). ` +
          `Complete your work locally; Bot-HQ handles PR creation. Only run a ` +
          `publish/push if the user explicitly authorized it this session, via ` +
          `the approval flow.`
      );
      process.exit(2); // blocking error — honored even under --dangerously-skip-permissions
    }
  }

  process.exit(0); // allow
}

main();
