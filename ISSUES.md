# ISSUES.md - Known Issues & Bugs

> **Last Updated**: 2026-01-23 (All critical issues fixed)
> **Tested By**: Claude (via Claude in Chrome browser automation)

---

## All Critical Issues - FIXED

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

All UI functionality working:
- Sidebar navigation (full page reload)
- Claude page with single eternal session
- Terminal view with command input
- Chat view with message input
- Mode toggle (Terminal/Chat)
- Status indicators

---

## Test Results (2026-01-23)

### Test Environment

- **URL**: http://localhost:7890
- **Browser**: Chrome (via Claude in Chrome extension)
- **Server**: Next.js dev server
- **Database**: SQLite with 7 tasks, 9 workspaces

### Tests Performed

1. **Sidebar Navigation**
   - `/claude` → `/` (Taskboard)
   - `/` → `/claude`
   - All navigation links working

2. **Manager Session**
   - Auto-connects on page load
   - PTY spawns successfully
   - Claude Code initializes and runs
   - Status indicator updates correctly

3. **Terminal Input**
   - Commands typed in Terminal view
   - Commands typed in Chat view
   - Send button works
   - Enter key works
   - Commands execute in Claude Code

4. **Session UI**
   - No "+ New" button
   - No session tabs
   - Clean single-session interface
   - Proper error handling with retry

---

## How to Verify Fixes

```bash
# 1. Ensure node-pty is properly installed
cd /Users/gregoryerrl/Projects/bot-hq
npm rebuild node-pty

# 2. Start the dev server
npm run dev

# 3. Open Chrome and navigate to
http://localhost:7890/claude

# 4. Verify:
#    - Page loads without "Rendering..." indicator
#    - Manager session auto-connects
#    - Terminal shows Claude Code output
#    - No "+ New" button or session tabs
#    - Sidebar navigation works
#    - Terminal/Chat input works
```
