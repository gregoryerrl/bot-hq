# Plugin System Phase 3: Remove GitHub from Core

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove all GitHub-specific code from core, making bot-hq work completely standalone.

**Architecture:** Delete GitHub libraries, sync functionality, and GitHub-specific UI. Agent prompts become generic (no GitHub issue references). All GitHub functionality will be restored via the GitHub plugin in Phase 5.

**Tech Stack:** Next.js, TypeScript, React, Drizzle ORM

---

## Task 1: Delete GitHub Library

**Files:**
- Delete: `src/lib/github/index.ts`
- Delete: `src/lib/github/types.ts`

**Step 1: Remove directory**

```bash
rm -rf src/lib/github
```

**Step 2: Verify deletion**

Run: `ls src/lib/github 2>&1`
Expected: No such file or directory

**Step 3: Commit**

```bash
git add -A && git commit -m "chore: remove GitHub library from core"
```

---

## Task 2: Delete Sync Library

**Files:**
- Delete: `src/lib/sync/index.ts`

**Step 1: Remove directory**

```bash
rm -rf src/lib/sync
```

**Step 2: Verify deletion**

Run: `ls src/lib/sync 2>&1`
Expected: No such file or directory

**Step 3: Commit**

```bash
git add -A && git commit -m "chore: remove GitHub sync library from core"
```

---

## Task 3: Delete Sync API Route

**Files:**
- Delete: `src/app/api/sync/route.ts`

**Step 1: Remove directory**

```bash
rm -rf src/app/api/sync
```

**Step 2: Verify deletion**

Run: `ls src/app/api/sync 2>&1`
Expected: No such file or directory

**Step 3: Commit**

```bash
git add -A && git commit -m "chore: remove sync API route"
```

---

## Task 4: Delete Clone API Route

**Files:**
- Delete: `src/app/api/workspaces/clone/route.ts`

**Step 1: Remove directory**

```bash
rm -rf src/app/api/workspaces/clone
```

**Step 2: Verify deletion**

Run: `ls src/app/api/workspaces/clone 2>&1`
Expected: No such file or directory

**Step 3: Commit**

```bash
git add -A && git commit -m "chore: remove workspace clone API route"
```

---

## Task 5: Remove Sync Button from Taskboard

**Files:**
- Delete: `src/components/taskboard/sync-button.tsx`
- Modify: `src/app/page.tsx`

**Step 1: Remove sync button component**

```bash
rm src/components/taskboard/sync-button.tsx
```

**Step 2: Update page.tsx - remove SyncButton import and usage**

Edit `src/app/page.tsx`:

Remove this line:
```tsx
import { SyncButton } from "@/components/taskboard/sync-button";
```

Remove this line from the JSX:
```tsx
<SyncButton />
```

**Step 3: Verify the file compiles**

Run: `npx tsc --noEmit src/app/page.tsx`

**Step 4: Commit**

```bash
git add -A && git commit -m "feat: remove sync button from taskboard"
```

---

## Task 6: Remove GitHub Fields from Add Workspace Dialog

**Files:**
- Modify: `src/components/settings/add-workspace-dialog.tsx`

**Step 1: Remove githubRemote state**

Remove:
```tsx
const [githubRemote, setGithubRemote] = useState("");
```

**Step 2: Remove cloning state and logic**

Remove:
```tsx
const [cloning, setCloning] = useState(false);
```

Remove the entire cloning block from handleSubmit:
```tsx
// If githubRemote is set, try auto-clone
if (githubRemote) {
  setCloning(true);
  const cloneRes = await fetch("/api/workspaces/clone", {
    ...
  });
  ...
  setCloning(false);
}
```

**Step 3: Remove githubRemote from API call**

Change:
```tsx
body: JSON.stringify({
  name,
  repoPath,
  githubRemote: githubRemote || null,
  buildCommand: buildCommand || null,
}),
```

To:
```tsx
body: JSON.stringify({
  name,
  repoPath,
  buildCommand: buildCommand || null,
}),
```

**Step 4: Remove githubRemote from resetState**

Remove:
```tsx
setGithubRemote("");
```

**Step 5: Remove GitHub Remote input field**

Remove entire block:
```tsx
<div className="space-y-2">
  <label className="text-sm font-medium">
    GitHub Remote (optional)
  </label>
  <Input
    value={githubRemote}
    onChange={(e) => setGithubRemote(e.target.value)}
    placeholder="owner/repo"
  />
</div>
```

**Step 6: Update button text**

Change:
```tsx
{cloning ? "Cloning..." : loading ? "Adding..." : "Add Workspace"}
```

To:
```tsx
{loading ? "Adding..." : "Add Workspace"}
```

**Step 7: Verify compilation**

Run: `npx tsc --noEmit`

**Step 8: Commit**

```bash
git add -A && git commit -m "feat: remove GitHub fields from add workspace dialog"
```

---

## Task 7: Update Workspace API Route

**Files:**
- Modify: `src/app/api/workspaces/route.ts`

**Step 1: Remove githubRemote from POST handler**

Change:
```tsx
const newWorkspace: NewWorkspace = {
  name: body.name,
  repoPath: body.repoPath,
  githubRemote: body.githubRemote || null,
  linkedDirs: body.linkedDirs ? JSON.stringify(body.linkedDirs) : null,
  buildCommand: body.buildCommand || null,
};
```

To:
```tsx
const newWorkspace: NewWorkspace = {
  name: body.name,
  repoPath: body.repoPath,
  linkedDirs: body.linkedDirs ? JSON.stringify(body.linkedDirs) : null,
  buildCommand: body.buildCommand || null,
};
```

**Step 2: Verify compilation**

Run: `npx tsc --noEmit`

**Step 3: Commit**

```bash
git add -A && git commit -m "chore: remove githubRemote from workspace API"
```

---

## Task 8: Update Agent Prompts to be Generic

**Files:**
- Modify: `src/lib/agents/claude-code.ts`

**Step 1: Update prompt for continuing work**

Change the prompt (around line 312-326):
```tsx
if (existingBranch && userInstructions) {
  // Continuing work with user feedback
  prompt = `You are continuing work on GitHub issue #${task.githubIssueNumber}: "${task.title}"

${task.description || "No description provided."}

You previously worked on this task and created branch: ${existingBranch}

The user has reviewed your work and requested changes:
${userInstructions}

Instructions:
1. Switch to the existing branch: git checkout ${existingBranch}
2. Address the user's feedback
3. Make commits as you work
4. Run the build (npm run build) before finishing
5. Do NOT push to remote - Bot-HQ will handle that after review`;
```

To:
```tsx
if (existingBranch && userInstructions) {
  // Continuing work with user feedback
  prompt = `You are continuing work on task: "${task.title}"

${task.description || "No description provided."}

You previously worked on this task and created branch: ${existingBranch}

The user has reviewed your work and requested changes:
${userInstructions}

Instructions:
1. Switch to the existing branch: git checkout ${existingBranch}
2. Address the user's feedback
3. Make commits as you work
4. Run the build (npm run build) before finishing
5. Do NOT push to remote - Bot-HQ will handle that after review`;
```

**Step 2: Update prompt for starting fresh**

Change (around line 327-344):
```tsx
} else {
  // Starting fresh
  prompt = `You are working on GitHub issue #${task.githubIssueNumber}: "${task.title}"

${task.description || "No description provided."}

Your task: Implement this feature completely.

Steps:
1. Create a feature branch: git checkout -b feature/${task.githubIssueNumber || "task"}-${task.id}
2. Implement the required changes with small, focused commits
3. Run tests and fix any issues
4. Run the build (npm run build) before finishing

Important:
- Make commits as you work
- Do NOT push to remote or create PRs - Bot-HQ will handle that after you finish
- Work autonomously - complete the full implementation`;
}
```

To:
```tsx
} else {
  // Starting fresh
  prompt = `You are working on task: "${task.title}"

${task.description || "No description provided."}

Your task: Implement this feature completely.

Steps:
1. Create a feature branch: git checkout -b feature/task-${task.id}
2. Implement the required changes with small, focused commits
3. Run tests and fix any issues
4. Run the build (npm run build) before finishing

Important:
- Make commits as you work
- Do NOT push to remote or create PRs - Bot-HQ will handle that after you finish
- Work autonomously - complete the full implementation`;
}
```

**Step 3: Verify compilation**

Run: `npx tsc --noEmit`

**Step 4: Commit**

```bash
git add -A && git commit -m "feat: update agent prompts to be generic (no GitHub references)"
```

---

## Task 9: Update Task Card to Remove GitHub-Specific UI

**Files:**
- Modify: `src/components/taskboard/task-card.tsx`

**Step 1: Remove GitHub issue number display**

Remove this block:
```tsx
{task.githubIssueNumber && (
  <span className="text-sm text-muted-foreground">
    #{task.githubIssueNumber}
  </span>
)}
```

**Step 2: Add generic task ID display**

Add this where the GitHub issue number was:
```tsx
<span className="text-sm text-muted-foreground">
  #{task.id}
</span>
```

**Step 3: Remove prUrl link (will be plugin responsibility)**

Remove:
```tsx
{task.prUrl && (
  <Button size="sm" variant="outline" asChild>
    <a href={task.prUrl} target="_blank" rel="noopener noreferrer">
      <ExternalLink className="h-4 w-4" />
    </a>
  </Button>
)}
```

**Step 4: Remove ExternalLink import if no longer used**

Check if ExternalLink is used elsewhere. If not, change:
```tsx
import { Play, ExternalLink } from "lucide-react";
```

To:
```tsx
import { Play } from "lucide-react";
```

**Step 5: Verify compilation**

Run: `npx tsc --noEmit`

**Step 6: Commit**

```bash
git add -A && git commit -m "feat: update task card to use generic task ID"
```

---

## Task 10: Update Task Assign Route Log Message

**Files:**
- Modify: `src/app/api/tasks/[id]/assign/route.ts`

**Step 1: Update log message**

Change:
```tsx
await db.insert(logs).values({
  workspaceId: task.workspaceId,
  taskId: task.id,
  type: "agent",
  message: `Task #${task.githubIssueNumber || task.id} queued for agent`,
});
```

To:
```tsx
await db.insert(logs).values({
  workspaceId: task.workspaceId,
  taskId: task.id,
  type: "agent",
  message: `Task #${task.id} queued for agent`,
});
```

**Step 2: Verify compilation**

Run: `npx tsc --noEmit`

**Step 3: Commit**

```bash
git add -A && git commit -m "chore: update task assign log to use generic task ID"
```

---

## Task 11: Remove "pr_created" State from Task State Enum

**Files:**
- Modify: `src/lib/db/schema.ts`

**Step 1: Update state enum**

The "pr_created" state is GitHub-specific. Remove it from the enum:

Change:
```tsx
state: text("state", {
  enum: [
    "new",
    "queued",
    "in_progress",
    "pending_review",
    "pr_created",
    "done",
  ],
})
```

To:
```tsx
state: text("state", {
  enum: [
    "new",
    "queued",
    "in_progress",
    "pending_review",
    "done",
  ],
})
```

**Step 2: Verify compilation**

Run: `npx tsc --noEmit`

**Step 3: Commit**

```bash
git add -A && git commit -m "chore: remove pr_created state from task enum"
```

---

## Task 12: Update Task Card State Colors

**Files:**
- Modify: `src/components/taskboard/task-card.tsx`

**Step 1: Remove pr_draft state color**

Change:
```tsx
const stateColors: Record<string, string> = {
  new: "bg-gray-500",
  queued: "bg-yellow-500",
  analyzing: "bg-blue-500",
  plan_ready: "bg-purple-500",
  in_progress: "bg-orange-500",
  pr_draft: "bg-green-500",
  done: "bg-green-700",
};
```

To:
```tsx
const stateColors: Record<string, string> = {
  new: "bg-gray-500",
  queued: "bg-yellow-500",
  analyzing: "bg-blue-500",
  plan_ready: "bg-purple-500",
  in_progress: "bg-orange-500",
  pending_review: "bg-green-500",
  done: "bg-green-700",
};
```

**Step 2: Remove pr_draft state label**

Change:
```tsx
const stateLabels: Record<string, string> = {
  new: "New",
  queued: "Queued",
  analyzing: "Analyzing",
  plan_ready: "Plan Ready",
  in_progress: "In Progress",
  pr_draft: "PR Draft",
  done: "Done",
};
```

To:
```tsx
const stateLabels: Record<string, string> = {
  new: "New",
  queued: "Queued",
  analyzing: "Analyzing",
  plan_ready: "Plan Ready",
  in_progress: "In Progress",
  pending_review: "Pending Review",
  done: "Done",
};
```

**Step 3: Verify compilation**

Run: `npx tsc --noEmit`

**Step 4: Commit**

```bash
git add -A && git commit -m "feat: update task card state colors for generic workflow"
```

---

## Task 13: Clean Up Draft PR Card Terminology

**Files:**
- Modify: `src/components/pending-board/draft-pr-card.tsx`

**Step 1: Rename file to approval-card.tsx**

Actually, let's keep the filename but update the component name and comments to be generic. The "draft PR" concept is replaced by "pending approval" which is already what we're using.

**Step 2: Update component comments if any**

Review the file and update any comments that reference GitHub/PR specifically.

**Step 3: Commit**

```bash
git add -A && git commit -m "chore: clean up draft PR card terminology"
```

---

## Task 14: Final Build Verification

**Step 1: Run full build**

```bash
npm run build
```

Expected: Build succeeds with no errors

**Step 2: Check for remaining GitHub references**

```bash
grep -r "github" src/ --include="*.ts" --include="*.tsx" | grep -v ".d.ts" | grep -v "node_modules"
```

Expected: No references to GitHub in core source code (only in comments/plan docs is OK)

**Step 3: Start dev server and test**

```bash
npm run dev
```

Test these flows:
1. Create a workspace (should NOT have GitHub Remote field)
2. Create a task manually
3. Assign and start agent on task
4. Agent completes and creates pending approval
5. Accept or Decline approval

**Step 4: Commit any final fixes**

```bash
git add -A && git commit -m "chore: phase 3 complete - bot-hq works standalone"
```

---

## Summary

After completing Phase 3:

1. ✅ No GitHub library in core (`src/lib/github/` deleted)
2. ✅ No sync functionality in core (`src/lib/sync/` deleted)
3. ✅ No sync button on taskboard
4. ✅ No GitHub fields in workspace creation
5. ✅ Agent prompts are generic (no GitHub references)
6. ✅ Task cards show generic task IDs
7. ✅ Task states are generic (no "pr_created")
8. ✅ Bot-hq works completely standalone

**Next:** Phase 4 will add plugin UI contributions (task badges, workspace settings tabs, sidebar tabs). Phase 5 will implement the GitHub plugin to restore all GitHub functionality.
