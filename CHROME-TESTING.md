# Chrome Testing Guide

> Definitive user flow testing guide for Bot-HQ using Claude in Chrome browser automation.

## Prerequisites

### 1. Start the Dev Server

```bash
cd /Users/gregoryerrl/Projects/bot-hq

# Ensure native modules are built correctly
npm rebuild better-sqlite3
npm rebuild node-pty

# Start the server
npm run dev
# or
npm run local
```

### 2. Verify Server is Running

```bash
curl -s http://localhost:7890/api/terminal/manager | jq .
```

Expected: `{"sessionId":"manager","exists":true,...}`

### 3. Chrome Extension Ready

Ensure Claude in Chrome extension is installed and connected.

---

## Test Flows

### Flow 1: Initial Page Load

**Purpose**: Verify the Claude page loads correctly with single session architecture.

**Steps**:
1. Navigate to `http://localhost:7890/claude`
2. Wait 3 seconds for connection

**Expected Results**:
- [ ] Header shows "Manager" with status indicator
- [ ] Status shows "Idle" (gray), "Working..." (green), or "Input needed" (yellow)
- [ ] Terminal/Chat toggle visible in top-right
- [ ] NO "+ New" button
- [ ] NO session tabs
- [ ] Terminal view shows Claude Code output
- [ ] No "Rendering..." indicator stuck on screen

**Failure Indicators**:
- "Failed to initialize manager session" error
- Blank screen
- Infinite loading spinner

---

### Flow 2: Sidebar Navigation

**Purpose**: Verify navigation between pages works correctly.

**Steps**:
1. Start on `/claude` page
2. Click "Taskboard" in sidebar
3. Wait for navigation
4. Click "Claude" in sidebar
5. Wait for navigation

**Expected Results**:
- [ ] URL changes to `/` (Taskboard)
- [ ] Taskboard content loads with task list
- [ ] URL changes back to `/claude`
- [ ] Manager session reconnects automatically

**Test All Navigation Links**:
| From | To | Click Target |
|------|-----|--------------|
| `/claude` | `/` | Taskboard |
| `/` | `/pending` | Pending |
| `/pending` | `/workspaces` | Workspaces |
| `/workspaces` | `/git-remote` | Git Remote |
| `/git-remote` | `/claude` | Claude |
| `/claude` | `/docs` | Docs |
| `/docs` | `/logs` | Logs |
| `/logs` | `/settings` | Settings |

---

### Flow 3: Terminal View Input

**Purpose**: Verify commands can be typed and executed in Terminal view.

**Steps**:
1. Navigate to `/claude`
2. Ensure "Terminal" mode is active (check top-right toggle)
3. Click in terminal area
4. Type a command: `status_overview`
5. Press Enter

**Expected Results**:
- [ ] Command appears in terminal
- [ ] Status changes to "Working..." (green indicator)
- [ ] Claude Code processes the command
- [ ] Response appears in terminal
- [ ] Status returns to "Idle" when complete

---

### Flow 4: Chat View Input

**Purpose**: Verify commands can be typed and sent in Chat view.

**Steps**:
1. Navigate to `/claude`
2. Click "Chat" toggle in top-right
3. Click in "Type a message..." input field
4. Type: `task_list`
5. Click Send button (or press Enter)

**Expected Results**:
- [ ] Input field is visible at bottom
- [ ] Text can be typed
- [ ] Send button is enabled when text is entered
- [ ] Command is sent to Claude Code
- [ ] Response appears in chat view
- [ ] Input field clears after send

---

### Flow 5: Mode Toggle

**Purpose**: Verify switching between Terminal and Chat views.

**Steps**:
1. Navigate to `/claude`
2. Note current mode (Terminal or Chat)
3. Click the other mode in toggle
4. Verify view changes
5. Click back to original mode

**Expected Results**:
- [ ] Terminal view shows xterm-style terminal
- [ ] Chat view shows parsed messages with input field
- [ ] Buffer content persists between switches
- [ ] Status indicator remains consistent

---

### Flow 6: Status Indicator Updates

**Purpose**: Verify status indicator reflects actual session state.

**Steps**:
1. Navigate to `/claude`
2. Observe status when idle
3. Send a command that takes time to process
4. Observe status during processing
5. Wait for completion

**Expected Status Colors**:
| State | Color | Text |
|-------|-------|------|
| Idle | Gray | "Idle" |
| Processing | Green | "Working..." |
| Needs Input | Yellow | "Input needed" |
| Permission Prompt | Yellow | "Permission needed" |
| Selection Menu | Yellow | "Selection needed" |

---

### Flow 7: Taskboard Functionality

**Purpose**: Verify taskboard displays and manages tasks correctly.

**Steps**:
1. Navigate to `/` (Taskboard)
2. Verify task list loads
3. Click "+ Create Task" button
4. Fill in task details
5. Submit task

**Expected Results**:
- [ ] Tasks display with status badges (Done, In Progress, etc.)
- [ ] Tasks show workspace labels
- [ ] "Request Changes" button visible on tasks
- [ ] Create task modal opens
- [ ] New task appears in list after creation

---

### Flow 8: Workspace List

**Purpose**: Verify workspaces page shows all configured workspaces.

**Steps**:
1. Navigate to `/workspaces`
2. Verify workspace list loads

**Expected Results**:
- [ ] All 9 workspaces displayed
- [ ] Each workspace shows name and path
- [ ] Build command visible if configured

---

### Flow 9: Logs Page

**Purpose**: Verify logs page shows system activity.

**Steps**:
1. Navigate to `/logs`
2. Verify logs load

**Expected Results**:
- [ ] Log entries displayed
- [ ] Timestamps visible
- [ ] Log types distinguishable (agent, error, etc.)

---

### Flow 10: Error Recovery

**Purpose**: Verify the system recovers from connection errors.

**Steps**:
1. Navigate to `/claude`
2. Stop the dev server (Ctrl+C)
3. Observe error state in browser
4. Restart dev server
5. Click "Retry Connection" button

**Expected Results**:
- [ ] Error message displayed when server stops
- [ ] "Retry Connection" button appears
- [ ] Connection re-establishes after retry
- [ ] Session resumes normally

---

## MCP Tool Verification

Use these MCP commands to verify the backend is working:

### Status Overview
```
mcp__bot-hq__status_overview
```
Expected: Returns running agents count, pending tasks, task counts by state.

### Task List
```
mcp__bot-hq__task_list
```
Expected: Returns all tasks with id, title, state, workspace.

### Workspace List
```
mcp__bot-hq__workspace_list
```
Expected: Returns all 9 workspaces with id, name, repoPath.

### Agent List
```
mcp__bot-hq__agent_list
```
Expected: Returns manager status and any running agents.

---

## Common Issues & Fixes

### Issue: "posix_spawnp failed" Error

**Cause**: node-pty native module corrupted or wrong version.

**Fix**:
```bash
rm -rf node_modules/node-pty
npm install node-pty
```

### Issue: "NODE_MODULE_VERSION mismatch" Error

**Cause**: Native module compiled for different Node.js version.

**Fix**:
```bash
npm rebuild better-sqlite3
npm rebuild node-pty
```

### Issue: Sidebar Navigation Not Working

**Cause**: SSE connections blocking Next.js routing.

**Status**: Fixed - using hard navigation (`window.location.assign()`).

### Issue: Chat Input Field Not Visible

**Cause**: Input hidden when status is `awaiting_input`.

**Status**: Fixed - added `awaiting_input` to showInput condition.

### Issue: Multiple Session Tabs Appearing

**Cause**: Old multi-session architecture.

**Status**: Fixed - rewritten to single eternal session.

### Issue: Chat View Shows Garbled Output

**Cause**: Terminal parser not fully stripping escape sequence fragments (`*Mi`, `*sg`, `+sg`, `*un`) and code block markers.

**Status**: Fixed - enhanced `src/lib/terminal-parser.ts` with `cleanTerminalArtifacts()` function and additional noise filters.

---

## Automated Test Script

For Claude in Chrome, use this sequence:

```
1. tabs_context_mcp (get tab context)
2. navigate to http://localhost:7890/claude
3. wait 3 seconds
4. screenshot (verify initial load)
5. click Taskboard in sidebar
6. wait 2 seconds
7. screenshot (verify navigation)
8. click Claude in sidebar
9. wait 2 seconds
10. click Chat toggle
11. screenshot (verify chat view)
12. click input field
13. type "status_overview"
14. click send button
15. wait 3 seconds
16. screenshot (verify command executed)
```

---

## Test Checklist Summary

### Critical Path (Must Pass)
- [ ] Page loads without errors
- [ ] Manager session connects automatically
- [ ] Sidebar navigation works
- [ ] Terminal input works
- [ ] Chat input works
- [ ] Status indicator updates correctly

### Secondary Features
- [ ] Mode toggle (Terminal/Chat)
- [ ] Taskboard displays tasks
- [ ] Workspace list shows all workspaces
- [ ] Logs page loads
- [ ] Error recovery works

### MCP Integration
- [ ] status_overview returns data
- [ ] task_list returns tasks
- [ ] workspace_list returns workspaces
- [ ] agent_list shows manager status

---

## Version History

| Date | Tester | Result | Notes |
|------|--------|--------|-------|
| 2026-01-23 | Claude | PASS | All critical issues fixed |

---

## Appendix: Browser Automation Commands

### Take Screenshot
```
mcp__claude-in-chrome__computer action=screenshot tabId=<id>
```

### Navigate
```
mcp__claude-in-chrome__navigate url=<url> tabId=<id>
```

### Click
```
mcp__claude-in-chrome__computer action=left_click coordinate=[x,y] tabId=<id>
```

### Type Text
```
mcp__claude-in-chrome__computer action=type text="<text>" tabId=<id>
```

### Wait
```
mcp__claude-in-chrome__computer action=wait duration=<seconds> tabId=<id>
```

### Read Page Elements
```
mcp__claude-in-chrome__read_page tabId=<id>
```

### Find Element
```
mcp__claude-in-chrome__find query="<description>" tabId=<id>
```
