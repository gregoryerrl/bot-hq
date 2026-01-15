# Unified Claude UI Design

## Overview

Merge the Terminal and Claude Chat tabs into a single "Claude" tab that connects to the PTY backend. Users can toggle between Terminal view (xterm.js) and Chat view (bubble UI) in real-time. Mobile users only see Chat view.

## Architecture

```
PTY Session (node-pty running Claude Code)
├── Raw output buffer (string)
├── SSE stream → both views receive data
├── Terminal view (xterm.js) ─┐
│                             ├── Toggle between these in real-time
└── Chat view (parsed bubbles)┘
```

Both views share the same PTY session and buffer. Switching is instant with no reconnection.

## Parsed Block Types

```typescript
type ParsedBlock =
  | { type: "assistant"; content: string }
  | { type: "user"; content: string }
  | { type: "code"; content: string; lang?: string }
  | { type: "tool"; name: string; output: string }
  | { type: "permission"; question: string; options: string[] }
  | { type: "thinking"; content: string }
```

## Permission Prompt Detection

Claude Code shows permission prompts like:

```
? Allow Claude to edit src/app.tsx?
❯ 1. Yes
  2. Yes, during this session
  3. No, and tell Claude what to do differently (esc)
```

**Detection patterns:**
- `?` at line start followed by question text
- `❯` or `>` indicating selected option
- Numbered options (1. 2. 3.)

**Button mapping:**
- "Yes" → sends `1` + Enter
- "Yes, during this session" → sends `2` + Enter
- "No, and tell Claude..." → sends `3` + Enter, then enables text input

## Chat UI States

| State | Terminal shows | Chat UI shows |
|-------|---------------|---------------|
| Idle | Cursor blinking | Text input enabled |
| Streaming | Text appearing | Typing indicator + bubbles |
| Permission prompt | 3 choices with `❯` | 3 buttons (input hidden) |
| "Tell Claude..." selected | Text input cursor | Text input enabled |
| Tool executing | Output streaming | Tool badge + output block |

## File Structure

### Files to create

```
src/
├── app/
│   └── claude/
│       └── page.tsx              # New unified route
├── components/
│   └── claude/
│       ├── claude-session.tsx    # Main wrapper with mode toggle
│       ├── terminal-view.tsx     # xterm.js rendering
│       ├── chat-view.tsx         # Chat bubble UI
│       ├── chat-message.tsx      # Message bubble component
│       ├── permission-prompt.tsx # Button group for permission choices
│       └── session-tabs.tsx      # Tab bar for multiple sessions
├── lib/
│   └── terminal-parser.ts        # Parse PTY output → structured blocks
└── hooks/
    └── use-session-buffer.ts     # Buffer PTY output for both views
```

### Files to remove

```
src/app/chat/page.tsx
src/app/terminal/page.tsx
src/components/claude-chat/*
src/components/terminal/*
```

### Files to modify

```
src/components/layout/sidebar.tsx  # Single "Claude" nav item
```

## Session Data Structure

```typescript
interface ClaudeSession {
  id: string;
  buffer: string;              // Raw PTY output
  parsedBlocks: ParsedBlock[]; // For chat view
  status: "idle" | "streaming" | "permission" | "input";
  currentPrompt?: PermissionPrompt;
}
```

## Mobile Handling

- Mode toggle hidden on mobile (always chat view)
- Session creation goes directly to chat mode
- Larger touch targets for buttons (min 44px)
- Input area sticky at bottom with keyboard handling

## Toggle Behavior

**Desktop:**
- Header shows `[Terminal | Chat]` toggle button
- Click to switch instantly between views
- Same session, same buffer - no reconnection

**Mobile:**
- No toggle visible
- Always renders chat view

## Session Creation Flow

1. User clicks "New Session"
2. PTY session created immediately via POST /api/terminal
3. SSE connection established
4. Desktop: Opens in default mode (or last-used preference)
5. Mobile: Opens directly in chat view
6. No modal/dialog for mode selection
