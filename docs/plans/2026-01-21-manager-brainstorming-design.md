# Manager Brainstorming Feature

## Overview

Give the bot-hq manager the capability to brainstorm with the user before spawning subagents. When a task is complex or ambiguous, the manager asks clarifying questions in the terminal. Bot-hq displays a notification that the manager is waiting for input.

## Flow

```
Task started
  → Manager evaluates complexity
  → Complex/ambiguous?
    → Yes: Manager asks questions in terminal using [AWAITING_INPUT] markers
    → Bot-hq parses terminal, shows notification
    → User answers in terminal
    → Manager compiles requirements
    → Manager spawns subagent with clear context
  → No: Manager spawns subagent directly
```

## Task State

Add new state to the task lifecycle:

```
new → queued → in_progress → awaiting_input → in_progress → done
                    ↓                              ↓
               needs_help                     needs_help
```

### New Database Fields

```typescript
// Add to tasks table
waitingQuestion: text       // The question manager is asking
waitingContext: text        // Conversation context so far
waitingSince: integer       // Timestamp when started waiting
```

## Terminal Marker Format

Manager outputs questions in a parseable format:

```
[AWAITING_INPUT]
Question: What approach do you prefer for the Git Remote feature?
Options:
1. Provider-agnostic with adapters
2. Provider tabs
3. Start GitHub-only, extensible later
[/AWAITING_INPUT]
```

For open-ended questions (no options):

```
[AWAITING_INPUT]
Question: What specific functionality do you need for the export feature?
[/AWAITING_INPUT]
```

## Parser Logic

1. Detect `[AWAITING_INPUT]` in terminal output stream
2. Extract question text and options (if present)
3. Update task state to `awaiting_input`
4. Store question/context in task record
5. When new substantive output appears (user answered), state returns to `in_progress`

## UI Components

### Task Card Badge

- State `awaiting_input` shows pulsing amber badge: "Awaiting Input"
- Hover/click reveals the question being asked
- Visual distinction from other states

### Global Notification Banner

- Fixed banner below nav, above content
- Text: "Manager is waiting for your input on Task #X: [question preview...]"
- Click expands to show full question + options
- If multiple tasks waiting: "2 tasks awaiting input" with dropdown
- Persists until task resumes (dismiss hides temporarily)

### Component Structure

```
src/components/notifications/
  └── awaiting-input-banner.tsx    # Global notification banner

src/components/taskboard/
  └── task-card.tsx                # Update to show awaiting_input badge
```

## Manager Behavior

### Complexity Detection Heuristics

Manager evaluates if brainstorming is needed:

- Task description is short but vague
- Keywords: "feature", "redesign", "implement", "architecture", "refactor"
- Missing: acceptance criteria, specific files, clear scope
- Ambiguous terms: "improve", "optimize", "clean up", "better"

### Brainstorming Guidelines

1. Ask one question at a time
2. Prefer multiple choice when possible (easier to answer)
3. Focus on: purpose, constraints, success criteria, approach preferences
4. Compile answers into clear requirements before spawning subagent

### Manager Prompt Addition

```
## Brainstorming Before Execution

When starting a task, evaluate if it needs clarification. Signs a task needs brainstorming:
- Vague or ambiguous description
- Multiple valid implementation approaches
- Missing acceptance criteria or success metrics
- Architectural decisions required

If brainstorming is needed:
1. Output your question using this format:
   [AWAITING_INPUT]
   Question: Your question here?
   Options:
   1. First option
   2. Second option
   [/AWAITING_INPUT]

2. Wait for the user to respond before continuing
3. Ask follow-up questions if needed (one at a time)
4. Once requirements are clear, compile them into a clear spec
5. Spawn subagent with the compiled requirements

Keep brainstorming focused - aim for 2-4 questions max.
```

## Implementation Files

| File | Change |
|------|--------|
| `src/lib/db/schema.ts` | Add waitingQuestion, waitingContext, waitingSince fields |
| `drizzle/migrations/` | New migration for schema changes |
| `src/lib/manager/persistent-manager.ts` | Add [AWAITING_INPUT] parser |
| `src/components/notifications/awaiting-input-banner.tsx` | New component |
| `src/components/taskboard/task-card.tsx` | Add awaiting_input badge |
| `src/app/layout.tsx` | Include notification banner |
| `src/lib/bot-hq/templates.ts` | Update manager prompt |
| `src/app/api/tasks/[id]/route.ts` | Handle new fields in PATCH |

## API Changes

### GET /api/tasks
- Returns new fields: waitingQuestion, waitingContext, waitingSince

### PATCH /api/tasks/:id
- Accepts new fields for updating waiting state

### GET /api/tasks/awaiting
- New endpoint: returns all tasks in awaiting_input state (for banner)
