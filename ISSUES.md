# ISSUES.md - Known Issues & Bugs

> **Last Updated**: 2026-01-23
> **Tested By**: Claude (via Claude in Chrome browser automation)

---

## Critical Issues

### 1. Browser UI Stuck in "Rendering" Loop

**Severity**: Critical
**Status**: Open
**Page**: `/claude` (Manager terminal page)

**Description**:
The Claude/manager page shows a perpetual "Rendering..." indicator in the bottom-left corner. This appears to be an infinite rendering loop that blocks all UI interactions.

**Symptoms**:
- "Rendering..." text never disappears
- Page becomes unresponsive to clicks
- Navigation links don't work
- Terminal input doesn't accept commands

**Suspected Cause**:
- Infinite `useEffect` loop or state update cycle
- Possible race condition in terminal/PTY state management
- Component re-rendering triggered by streaming data

**Files to Investigate**:
- `src/app/claude/page.tsx`
- `src/components/claude-session.tsx`
- `src/components/terminal-view.tsx`

**Reproduction**:
1. Navigate to `http://localhost:7890/claude`
2. Observe "Rendering..." indicator in bottom-left
3. Try clicking any sidebar link - nothing happens

---

### 2. Sidebar Navigation Blocked

**Severity**: Critical
**Status**: Open
**Page**: All pages (sidebar component)

**Description**:
Clicking on sidebar navigation links does not navigate to the target page. The links have correct `href` attributes but clicks don't trigger navigation.

**Symptoms**:
- Sidebar links are visible and have correct hrefs
- Clicking links produces no navigation
- URL in browser doesn't change
- Page content remains the same

**Suspected Cause**:
- Event propagation being stopped by parent component
- Terminal/chat components capturing click events
- Next.js client-side routing conflict
- Z-index issue with overlay element

**Files to Investigate**:
- `src/components/sidebar.tsx`
- `src/app/claude/page.tsx` (may have event handlers blocking)
- `src/app/layout.tsx`

**Reproduction**:
1. Be on `/claude` page
2. Click "Taskboard" in sidebar
3. Nothing happens - still on `/claude`

---

### 3. Manager Session Not Starting

**Severity**: Critical
**Status**: Open
**Page**: `/claude`

**Description**:
The "+ New" button shows a loading spinner but never successfully starts a new manager session. The MCP `agent_start` tool returns "Manager is not running" error.

**Symptoms**:
- "+ New" button shows loading spinner indefinitely
- `agent_list` MCP tool shows `managerStatus: "stopped"`
- `agent_start` returns error: "Manager is not running"
- No PTY session is created

**Suspected Cause**:
- Manager startup logic not implemented or broken
- PTY spawn failing silently
- Missing initialization in persistent manager

**Files to Investigate**:
- `src/lib/manager/persistent-manager.ts`
- `src/lib/pty-manager.ts`
- `src/app/api/terminal/manager/route.ts`

**Reproduction**:
1. Navigate to `/claude`
2. Click "+ New" button
3. Button shows loading but never completes
4. Run MCP tool `agent_list` - shows manager stopped

---

## Medium Issues

### 4. Terminal Input Unresponsive

**Severity**: Medium
**Status**: Open
**Page**: `/claude`

**Description**:
The terminal input field on the Claude page doesn't respond to Enter key or send button clicks. Commands typed in the input are not sent to the manager.

**Symptoms**:
- Text can be typed in input field
- Pressing Enter does nothing
- Clicking "send" button does nothing
- "list tasks" command visible but not executed

**Suspected Cause**:
- Related to the rendering loop issue (#1)
- Event handlers not attached properly
- Manager session not running (#3)

**Files to Investigate**:
- Terminal input component
- `src/app/api/terminal/manager/route.ts` (POST handler)

**Reproduction**:
1. Navigate to `/claude`
2. Click on terminal input field
3. Type any command
4. Press Enter or click "send"
5. Nothing happens

---

### 5. URL Navigation Fails on New Tabs

**Severity**: Medium
**Status**: Open
**Context**: Browser automation testing

**Description**:
When creating new browser tabs and attempting to navigate via the `navigate` tool or address bar, the navigation doesn't complete. New tabs remain on `chrome://newtab/`.

**Symptoms**:
- `tabs_create_mcp` creates tab successfully
- `navigate` tool reports success but URL doesn't change
- Tab title remains "New Tab"
- Cannot access localhost from new tabs

**Suspected Cause**:
- Browser automation tool limitation
- Chrome extension permission issue
- Tab not fully initialized before navigation

**Workaround**:
Use existing tabs that are already on the target domain.

---

## Verified Working

### MCP API Layer

All MCP tools tested and working correctly:

| Tool | Status | Notes |
|------|--------|-------|
| `task_list` | ✅ Working | Returns all tasks with filters |
| `task_get` | ✅ Working | Returns full task details |
| `task_create` | ✅ Working | Creates tasks successfully |
| `task_update` | ✅ Working | Updates state, priority, notes |
| `task_assign` | ✅ Working | Moves new → queued |
| `workspace_list` | ✅ Working | Lists all 9 workspaces |
| `status_overview` | ✅ Working | Returns correct counts |
| `agent_list` | ✅ Working | Shows manager status |
| `agent_start` | ⚠️ Blocked | Works but manager not running |

### Database Operations

All database operations working:
- Task CRUD operations
- Workspace management
- State transitions
- Timestamp updates

---

## Testing Notes

### What Was Tested

1. **Browser UI Navigation**
   - Sidebar link clicks
   - Direct URL navigation
   - Keyboard shortcuts (Cmd+L, Cmd+R)
   - JavaScript-based navigation

2. **Terminal Interaction**
   - Input field focus
   - Typing commands
   - Enter key submission
   - Send button clicks

3. **Manager Session**
   - "+ New" button
   - MCP agent tools
   - Session status checks

4. **MCP API**
   - All task tools
   - All workspace tools
   - Monitoring tools

### Test Environment

- **URL**: http://localhost:7890
- **Browser**: Chrome (via Claude in Chrome extension)
- **Server**: Next.js dev server running
- **Database**: SQLite with 7 tasks, 9 workspaces

---

## Recommended Fix Priority

1. **First**: Fix the rendering loop (#1) - this likely causes #2 and #4
2. **Second**: Fix manager startup (#3) - core functionality blocked
3. **Third**: Verify sidebar navigation (#2) after #1 is fixed
4. **Fourth**: Terminal input (#4) should work after #1 and #3

---

## Fix Applied (2026-01-23)

### Issue #2: Sidebar Navigation - FIXED & VERIFIED ✅

**Root Cause**: SSE (Server-Sent Events) connections from `NotificationProvider` and `ClaudeSession` components were interfering with Next.js client-side navigation. When active EventSource connections exist, Next.js App Router navigation can become blocked.

**Solution**: Changed sidebar navigation to use hard navigation (`window.location.assign()`) instead of Next.js client-side routing. This forces a full page reload which properly cleans up SSE connections.

**Files Modified**:
- `src/components/layout/sidebar.tsx` - Added onClick handler with `window.location.assign()`
- `src/components/layout/mobile-nav.tsx` - Same fix for mobile navigation
- `src/components/notifications/awaiting-input-banner.tsx` - Changed `<Link>` to `<a>` tags

**Trade-off**: Navigation is now slightly slower (full page reload) but reliable. A better long-term fix would be to properly close SSE connections before navigation.

**Verified**: 2026-01-23 via Chrome browser automation
- `/claude` → `/` (Taskboard) ✅
- `/` → `/logs` ✅
- `/logs` → `/claude` ✅

### Issue #4: Terminal Input - NOT REPRODUCED

During testing, terminal input was working correctly. The "list tasks" command was successfully submitted and Claude Code responded.

### Issue #1 & #3: Rendering Loop & Manager Not Starting - NOT REPRODUCED

Did not observe the "Rendering..." indicator or manager startup issues during testing. The manager session appeared to be running with active terminal output.

---

## How to Reproduce Full Test

```bash
# 1. Start the dev server
cd /Users/gregoryerrl/Projects/bot-hq
npm run dev

# 2. Open Chrome and navigate to
http://localhost:7890/claude

# 3. Observe issues:
#    - "Rendering..." indicator stuck
#    - Sidebar clicks don't navigate
#    - Terminal input doesn't work
#    - "+ New" button loads forever

# 4. Test MCP tools (these work):
#    Use Claude Code with bot-hq MCP server
#    Run: status_overview, task_list, workspace_list
```
