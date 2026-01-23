# ISSUES.md - Known Issues & Bugs

> **Last Updated**: 2026-01-23 16:00 PHT
> **Tested By**: Claude (via Claude in Chrome browser automation)

---

## All Issues Resolved

No critical issues currently open.

---

## Previously Fixed Issues

### 7. Taskboard Shows "No tasks found" Despite Tasks Existing - FIXED

**Severity**: Critical
**Status**: FIXED
**Page**: `/` (Taskboard)

**Root Cause**: The `better-sqlite3` native module was compiled for a different Node.js version (NODE_MODULE_VERSION 137 vs required 127), causing database queries to fail silently.

**Solution**: Rebuilt the native module:
```bash
npm rebuild better-sqlite3
```

**Verified**: 2026-01-23 - Taskboard now displays all 7 tasks correctly.

---

### 8. Workspaces Page Shows "No workspaces configured" Despite Workspaces Existing - FIXED

**Severity**: Critical
**Status**: FIXED
**Page**: `/workspaces`

**Root Cause**: Same as Issue #7 - `better-sqlite3` native module version mismatch.

**Solution**: Same fix as Issue #7 - `npm rebuild better-sqlite3`

**Verified**: 2026-01-23 - Workspaces page now displays all 9 workspaces correctly.

---

### 9. Chat View Renders Garbled Terminal Output - FIXED

**Severity**: Medium
**Status**: FIXED
**Page**: `/claude` (Chat view)

**Root Cause**: The terminal parser (`src/lib/terminal-parser.ts`) wasn't fully cleaning escape sequence fragments and terminal UI artifacts from the buffer.

**Solution**: Enhanced the terminal parser with:
1. Added `cleanTerminalArtifacts()` function to strip remaining escape sequences after `stripAnsi()`
2. Added filters for partial escape fragments like `*Mi`, `*sg`, `+sg`, `*un`
3. Added filters for code block markers (`\`\`\``) and ellipsis lines
4. Enhanced `isNoiseeLine()` to catch more terminal UI noise patterns

**Files Modified**:
- `src/lib/terminal-parser.ts`

**Verified**: 2026-01-23 - Chat view now renders clean, readable output.

---

### 1. Browser UI Stuck in "Rendering" Loop - FIXED

**Severity**: Critical
**Status**: FIXED
**Page**: `/claude` (Manager terminal page)

**Resolution**: Issue was not reproduced after the session management simplification (Issue #5 fix). The rendering loop was likely caused by the complex multi-session state management. The simplified single-session architecture eliminated the issue.

---

### 2. Sidebar Navigation Blocked - FIXED

**Severity**: Critical
**Status**: FIXED
**Page**: All pages (sidebar component)

**Root Cause**: SSE (Server-Sent Events) connections from `NotificationProvider` and `ClaudeSession` components were interfering with Next.js client-side navigation.

**Solution**: Changed sidebar navigation to use hard navigation (`window.location.assign()`) instead of Next.js client-side routing.

**Files Modified**:
- `src/components/layout/sidebar.tsx`
- `src/components/layout/mobile-nav.tsx`
- `src/components/notifications/awaiting-input-banner.tsx`

**Verified**: 2026-01-23 via Chrome browser automation

---

### 3. Manager Session Not Starting - FIXED

**Severity**: Critical
**Status**: FIXED
**Page**: `/claude`

**Root Cause**: The `node-pty` native module was corrupted or compiled for a different Node.js version, causing `posix_spawnp failed` errors when attempting to spawn the PTY process.

**Solution**: Reinstalled `node-pty` module:
```bash
rm -rf node_modules/node-pty
npm install node-pty
```

**Verified**: 2026-01-23 - Manager session now starts successfully and Claude Code runs properly.

---

## Medium Issues - FIXED

### 4. Terminal Input Unresponsive - FIXED

**Severity**: Medium
**Status**: FIXED
**Page**: `/claude`

**Root Cause**: The Chat view input field was not shown when status was `awaiting_input`. The `showInput` condition only checked for `idle` and `input` statuses.

**Solution**: Updated `src/components/claude/chat-view.tsx` to include `awaiting_input` in the `showInput` condition:
```typescript
const showInput = status === "idle" || status === "input" || status === "awaiting_input" ||
    (permissionPrompt && isTellClaudeSelected(permissionPrompt));
```

**Verified**: 2026-01-23 - Commands can now be typed and sent successfully.

---

### 5. Claude Page Has Unnecessary Session Management UI - FIXED

**Severity**: Medium
**Status**: FIXED
**Page**: `/claude`

**Root Cause**: The original implementation supported multiple sessions with a `+ New` button, session tabs, and session switching - but the architecture should only support one eternal Claude Code session.

**Solution**: Completely rewrote `src/components/claude/claude-session.tsx` to:
- Remove multi-session support
- Remove `+ New` button
- Remove session tabs
- Auto-connect to the single manager session on page load
- Show clean status indicator (Manager + Idle/Working/Input needed)
- Add "Retry Connection" button for error recovery

**Files Modified**:
- `src/components/claude/claude-session.tsx` - Complete rewrite for single session
- `src/components/claude/chat-view.tsx` - Fixed input visibility

**New UI Features**:
- Clean header showing "Manager" with status indicator (green=working, yellow=input needed, gray=idle)
- Terminal/Chat mode toggle
- Auto-reconnect on page load
- Error state with retry button

**Verified**: 2026-01-23 via Chrome browser automation

---

### 6. URL Navigation Fails on New Tabs

**Severity**: Medium
**Status**: Open (Browser automation limitation, not a bot-hq issue)
**Context**: Browser automation testing

**Description**: This is a Chrome extension/browser automation limitation, not a bot-hq issue.

**Workaround**: Use existing tabs that are already on the target domain.

---

## Verified Working

### MCP API Layer

All MCP tools tested and working correctly:

| Tool | Status | Notes |
|------|--------|-------|
| `task_list` | Working | Returns all tasks with filters |
| `task_get` | Working | Returns full task details |
| `task_create` | Working | Creates tasks successfully |
| `task_update` | Working | Updates state, priority, notes |
| `task_assign` | Working | Moves new → queued |
| `workspace_list` | Working | Lists all 9 workspaces |
| `status_overview` | Working | Returns correct counts |
| `agent_list` | Working | Shows manager status |
| `agent_start` | Working | Manager starts successfully |

### Database Operations

All database operations working:
- Task CRUD operations
- Workspace management
- State transitions
- Timestamp updates

### Browser UI

All working (verified 2026-01-23 16:00 PHT):
- ✅ Sidebar navigation (full page reload)
- ✅ Claude page with single eternal session
- ✅ Terminal view with command input
- ✅ Chat view with message input and clean rendering
- ✅ Mode toggle (Terminal/Chat)
- ✅ Status indicators
- ✅ Taskboard - shows all 7 tasks correctly
- ✅ Workspaces - shows all 9 workspaces correctly

---

## Test Results (2026-01-23 16:00 PHT)

### Test Environment

- **URL**: http://localhost:7890
- **Browser**: Chrome (via Claude in Chrome extension)
- **Server**: Next.js dev server
- **Database**: SQLite with 7 tasks, 9 workspaces

### Tests Performed

1. **Sidebar Navigation** ✅ PASS
   - All navigation links working via hard reload

2. **Manager Session** ✅ PASS
   - Auto-connects on page load
   - PTY spawns successfully
   - Claude Code initializes and runs
   - Status indicator shows "Input needed" (yellow)

3. **Terminal View Input** ✅ PASS
   - Commands typed and displayed
   - Commands execute in Claude Code
   - Output displays correctly with tables

4. **Chat View Input** ✅ PASS
   - Input field visible and works
   - Commands can be typed and sent
   - Send button works
   - Output renders clean and readable

5. **Session UI** ✅ PASS
   - No "+ New" button
   - No session tabs
   - Clean single-session interface
   - Terminal/Chat toggle works

6. **Taskboard** ✅ PASS
   - Shows all 7 tasks correctly
   - Task cards display with proper styling

7. **Workspaces Page** ✅ PASS
   - Shows all 9 workspaces correctly
   - Workspace cards display with paths

8. **Git Remote Page** ✅ PASS
   - Page loads correctly
   - Tabs visible (Remotes, Issues, Clone)
   - Add Remote button present

9. **Logs Page** ✅ PASS
   - Page loads correctly

10. **Pending/Review Page** ✅ PASS
    - Shows git-native review migration message

---

## How to Verify Fixes

```bash
# 1. Ensure native modules are properly installed
cd /Users/gregoryerrl/Projects/bot-hq
npm rebuild node-pty
npm rebuild better-sqlite3

# 2. Start the dev server
npm run dev

# 3. Open Chrome and navigate to
http://localhost:7890/

# 4. Verify:
#    - Taskboard shows all tasks
#    - Workspaces page shows all workspaces
#    - Claude page loads with manager session
#    - Chat view renders clean output
#    - Terminal view shows formatted tables
#    - All navigation works
```
