import stripAnsi from "strip-ansi";

export type ParsedBlock =
  | { type: "assistant"; content: string }
  | { type: "user"; content: string }
  | { type: "code"; content: string; lang?: string }
  | { type: "tool"; name: string; output: string }
  | { type: "permission"; question: string; options: string[] }
  | { type: "thinking"; content: string }
  | { type: "selection-menu"; menu: SelectionMenu };

export interface PermissionPrompt {
  question: string;
  options: string[];
  selectedIndex: number;
}

export interface SelectionMenuItem {
  label: string;
  description?: string;
  isSelected: boolean;
}

export interface SelectionMenu {
  title: string;
  items: SelectionMenuItem[];
  selectedIndex: number;
  hasSearch: boolean;
  instructions?: string;
}

export interface AwaitingInputPrompt {
  taskId?: number;
  question: string;
  options: string[];
  context?: string;
}

// Lines that should never appear as permission options
const INVALID_OPTION_PATTERNS = [
  /^(Esc|Tab|Enter|Space)\s+to\s+/i,
  /to\s+(cancel|add|select|navigate|confirm)/i,
  /·\s*(Tab|Esc|Enter)/i,
  /↑↓/,
  /↵/,
  /shortcuts/i,
  /ctrl\+/i,
  /^[└├│┃┆┊]/,                        // Box drawing prefix
  /^Tip:/i,
  /install-github-app/i,
  /@claude.*github/i,
  /^\s*$/,                             // Empty
  /^[─┌┐└┘│├┤┬┴┼╭╮╯╰\s|]+$/,         // Box drawing only
  /for\s+shortcuts/i,
  /press\s+\w+\s+to/i,
  /Run\s+\//,                          // Run /command suggestions
  /^Search/i,                          // Search box
];

function isValidOption(text: string): boolean {
  if (!text || text.length < 2 || text.length > 100) return false;
  return !INVALID_OPTION_PATTERNS.some(pattern => pattern.test(text));
}

// Detect if output ends with a permission prompt
export function detectPermissionPrompt(buffer: string): PermissionPrompt | null {
  const clean = stripAnsi(buffer);
  const lines = clean.split("\n").filter((l) => l.trim());

  // Look for pattern: ? question followed by numbered options with ❯ indicator
  // Find the last question line (within last 30 lines)
  const searchLines = lines.slice(-30);
  let questionIndex = -1;
  for (let i = searchLines.length - 1; i >= 0; i--) {
    if (searchLines[i].trim().startsWith("?")) {
      questionIndex = i;
      break;
    }
  }

  if (questionIndex === -1) return null;

  const question = searchLines[questionIndex].replace(/^\?\s*/, "").trim();
  const options: string[] = [];
  let selectedIndex = 0;

  // Parse options after the question (only next 10 lines max)
  const maxOptions = Math.min(questionIndex + 10, searchLines.length);
  for (let i = questionIndex + 1; i < maxOptions; i++) {
    const line = searchLines[i];
    const trimmed = line.trim();

    // Stop if we hit another question or empty section
    if (trimmed.startsWith("?") && i > questionIndex + 1) break;

    // Check if this looks like an option (has selection indicator or number prefix)
    const hasSelector = line.includes("❯") || /^\s*>\s/.test(line);
    const hasNumber = /^\s*\d+\./.test(trimmed) || /^[❯>]\s*\d+\./.test(trimmed);

    // Extract the option text
    let optionText = trimmed
      .replace(/^[❯>]\s*/, '')
      .replace(/^\d+\.\s*/, '')
      .trim();

    // Validate the option
    if (!isValidOption(optionText)) continue;

    // Only add if it looks like a real option
    if (hasSelector || hasNumber || options.length < 5) {
      if (hasSelector) selectedIndex = options.length;

      // Avoid duplicates
      if (!options.includes(optionText)) {
        options.push(optionText);
      }
    }
  }

  if (options.length < 2) return null;

  return { question, options, selectedIndex };
}

// Helper to create a block with proper typing
function createBlock(type: string, content: string, toolName?: string): ParsedBlock | null {
  const trimmed = content.trim();
  if (!trimmed) return null;

  // Filter out blocks that are just prompt characters or too short
  if (/^[❯>●]\s*$/.test(trimmed)) return null;
  if (trimmed.length < 3) return null;

  // Filter out blocks that are just box-drawing or decorative
  if (/^[─┌┐└┘│├┤┬┴┼╭╮╯╰└├│┃┆┊\s|]+$/.test(trimmed)) return null;

  // Filter out tip lines
  if (/^[└├│┃┆┊]?\s*Tip:/i.test(trimmed)) return null;
  if (/install-github-app/i.test(trimmed)) return null;

  switch (type) {
    case "assistant":
      return { type: "assistant", content: trimmed };
    case "code":
      return { type: "code", content: trimmed };
    case "tool":
      return { type: "tool", name: toolName || "Tool", output: trimmed };
    case "user":
      return { type: "user", content: trimmed };
    case "thinking":
      return { type: "thinking", content: trimmed };
    default:
      return { type: "assistant", content: trimmed };
  }
}

// Lines that should be filtered from chat output (terminal UI noise)
const NOISE_PATTERNS = [
  // Keyboard shortcuts and instructions
  /ctrl\+[a-z]\s+to\s+/i,
  /\?\s*for\s+shortcuts/i,
  /^\/\w+\s+for\s+/i,
  /press\s+\w+\s+to\s+/i,
  /↑↓\s*to\s+navigate/i,
  /enter\s+to\s+select/i,
  /escape\s+to\s+/i,
  /esc\s+to\s+cancel/i,
  /tab\s+to\s+add/i,
  /·\s*(tab|esc|enter)/i,
  /^\s*↵\s*(send)?\s*$/i,

  // Prompt characters and selection indicators
  /^\s*>\s*$/,
  /^\s*❯\s*$/,
  /^❯\s*\d+\./,
  /^❯\s+$/,
  /^\s*\d+\.\s*(Yes|No|Always|Never|Once)\s*$/i,
  /^[❯>]\s*\d+\.\s*(Yes|No|Always|Never|Once)/i,

  // Box drawing and decorative lines
  /^[─┌┐└┘│├┤┬┴┼╭╮╯╰]+$/,
  /^[\s│|┃]+$/,
  /^[-─═]+$/,
  /^[╔╗╚╝║═]+$/,
  /^[└├│┃┆┊]\s*$/,
  /^\s*\|\s*$/,

  // Welcome screen and tips
  /Tips for getting started/i,
  /Recent activity/i,
  /No recent activity/i,
  /Run\s+\/init\s+to\s+create/i,
  /Welcome\s+back/i,
  /Claude\s+Code\s+v\d/i,
  /Try\s+"how\s+does/i,

  // Tip messages (comprehensive)
  /^[└├│┃┆┊]\s*Tip:/i,
  /^Tip:\s*/i,
  /Tip:\s*Run\s+\//i,
  /install-github-app/i,
  /@claude.*Github.*issues/i,
  /tag\s+@claude\s+right\s+from/i,

  // Other UI elements
  /^Search/i,
  /^\s*\(\d+\s+other\s+sessions?\)/i,
  /^\d+\s+(hour|day|week|month|year)s?\s+ago/i,
  /\d+\s+messages?\s*$/i,

  // Claude Code suggestions and prompts
  /^Try\s+"/i,                        // Try "..." suggestions
  /^Try\s+".*"\s*$/i,                 // Full try suggestions
  /create\s+a\s+util.*\.py/i,         // Common suggestions
  /help\s+me\s+start\s+a\s+new/i,     // Help me start suggestions
  /^❯\s*Try\s+/i,                     // Try with prompt indicator
  /send$/i,                           // "send" button text
];

function isNoiseeLine(line: string): boolean {
  const trimmed = line.trim();
  if (!trimmed) return true; // Skip empty lines

  // Skip very short lines (likely UI artifacts)
  if (trimmed.length < 3) return true;

  // Skip garbled escape sequences
  if (/\^\[\[/.test(trimmed)) return true;
  if (/\[\[O/.test(trimmed)) return true;
  if (/^[a-z]\^/.test(trimmed)) return true;

  // Skip lines that are just prompt characters
  if (/^[❯>●]\s*$/.test(trimmed)) return true;

  // Skip lines that are mostly box-drawing or decorative
  if (/^[─┌┐└┘│├┤┬┴┼╭╮╯╰\s|└├│┃┆┊]+$/.test(trimmed)) return true;

  // Skip lines that start with box-drawing chars
  if (/^[└├│┃┆┊]/.test(trimmed)) return true;

  // Skip ANY line containing "Tip:" anywhere
  if (/Tip:/i.test(trimmed)) return true;

  // Skip lines containing /agents or specific suggestions
  if (/\/agents/i.test(trimmed)) return true;
  if (/optimize\s+specific\s+tasks/i.test(trimmed)) return true;
  if (/Software\s+Architect/i.test(trimmed)) return true;
  if (/Code\s+Writer/i.test(trimmed)) return true;
  if (/Code\s+Reviewer/i.test(trimmed)) return true;

  // Skip selection option lines (numbered choices)
  if (/^[❯>]?\s*\d+\.\s*(Yes|No|Always|Never|Once|Allow|Deny|Skip)/i.test(trimmed)) return true;

  // Skip Claude Code suggestion prompts with ❯
  if (/^❯\s*(Try|help me|create|what|how)/i.test(trimmed)) return true;

  // Skip "↵ send" and similar
  if (/↵.*send/i.test(trimmed)) return true;

  return NOISE_PATTERNS.some(pattern => pattern.test(trimmed));
}

// Parse full buffer into blocks for chat view
export function parseTerminalOutput(buffer: string): ParsedBlock[] {
  const clean = stripAnsi(buffer);
  const blocks: ParsedBlock[] = [];

  // Split by common delimiters
  const lines = clean.split("\n");
  let currentBlock: string[] = [];
  let currentType = "assistant";
  let currentToolName = "";

  const flushBlock = () => {
    if (currentBlock.length > 0) {
      const block = createBlock(currentType, currentBlock.join("\n"), currentToolName);
      if (block) blocks.push(block);
      currentBlock = [];
    }
  };

  const seenLines = new Set<string>();

  for (const line of lines) {
    // Skip terminal UI noise lines in chat view
    if (isNoiseeLine(line)) {
      continue;
    }

    // Normalize line for duplicate detection
    const normalizedLine = line.replace(/^[❯>●\s]+/, '').trim().toLowerCase();

    // Skip duplicate lines within the same parse
    if (normalizedLine.length > 3 && seenLines.has(normalizedLine)) {
      continue;
    }
    if (normalizedLine.length > 3) {
      seenLines.add(normalizedLine);
    }

    // Detect user input (lines starting with > or after prompt)
    if (line.match(/^>\s/) || line.match(/^❯\s*\d+\./)) {
      flushBlock();
      continue;
    }

    // Detect code blocks (lines with consistent indentation or ````)
    if (line.startsWith("```") || line.match(/^\s{4,}/)) {
      if (currentType !== "code") {
        flushBlock();
        currentType = "code";
      }
    }

    // Detect tool output (common patterns)
    const toolMatch = line.match(/^(Read|Write|Edit|Bash|Glob|Grep):/i);
    if (toolMatch) {
      flushBlock();
      currentType = "tool";
      currentToolName = toolMatch[1];
    }

    currentBlock.push(line);
  }

  // Flush remaining
  flushBlock();

  // Deduplicate and filter blocks
  const deduped: ParsedBlock[] = [];
  const seen = new Set<string>();
  const recentContent: string[] = [];

  // Additional patterns to filter at block level
  const BLOCK_FILTER_PATTERNS = [
    /Tip:/i,                            // Any tip message
    /install-github-app/i,
    /tag\s+@claude/i,
    /Github\s+issues\s+and\s+PRs/i,
    /^\d+\.\s*(Yes|No|Always|Never)/i,
    /^[❯>]\s*(Try|help me|create)/i,
    /^[└├│┃┆┊]/,
    /^Try\s+"/i,
    /↵.*send/i,
    /help\s+me\s+start\s+a\s+new/i,
    /\/agents/i,                        // /agents suggestions
    /optimize\s+specific\s+tasks/i,
    /Software\s+Architect/i,
    /Code\s+Writer/i,
    /Code\s+Reviewer/i,
    /Use\s+\/\w+\s+to/i,               // Use /command to suggestions
  ];

  let lastContent = "";

  for (const block of blocks) {
    const content = 'content' in block ? block.content :
                    'output' in block ? block.output : '';

    // Normalize content for comparison
    const normalized = content
      .replace(/^[└├│┃┆┊\s❯>●]+/gm, '')  // Strip leading decorative chars
      .replace(/\s+/g, ' ')               // Collapse whitespace
      .trim();

    // Skip empty or near-empty normalized content
    if (!normalized || normalized.length < 5) continue;

    // Skip if matches any block filter pattern
    if (BLOCK_FILTER_PATTERNS.some(p => p.test(normalized))) continue;

    // Create a simplified key for duplicate detection
    const simpleKey = normalized.toLowerCase().slice(0, 100);

    // Skip if same as last block (consecutive duplicate)
    if (simpleKey === lastContent) continue;

    // Skip if we've seen this exact content
    if (seen.has(simpleKey)) continue;

    // Skip if content is very similar to what we just showed
    const isSimilar = recentContent.some(recent => {
      // Exact match
      if (recent === simpleKey) return true;
      // Very short content - require exact match
      if (recent.length < 15 || simpleKey.length < 15) return recent === simpleKey;
      // Longer content - check for substring
      return recent.includes(simpleKey) || simpleKey.includes(recent);
    });
    if (isSimilar) continue;

    // Track this content
    lastContent = simpleKey;
    seen.add(simpleKey);
    recentContent.push(simpleKey);

    // Keep reasonable history
    if (seen.size > 100) {
      const first = seen.values().next().value;
      if (first) seen.delete(first);
    }
    if (recentContent.length > 50) {
      recentContent.shift();
    }

    deduped.push(block);
  }

  return deduped;
}

// Check if the "tell claude" option is selected
export function isTellClaudeSelected(prompt: PermissionPrompt): boolean {
  const selected = prompt.options[prompt.selectedIndex]?.toLowerCase() || "";
  return selected.includes("tell claude") || selected.includes("do differently") || selected.includes("feedback");
}

// Detect if output ends with an interactive selection menu (like /resume picker)
export function detectSelectionMenu(buffer: string): SelectionMenu | null {
  const clean = stripAnsi(buffer);
  const lines = clean.split("\n").filter(l => l.trim());

  // Look for "Resume Session" pattern - the /resume command picker
  const lastLines = lines.slice(-20); // Check last 20 lines

  let titleIndex = -1;
  let title = "";

  // Find title line (Resume Session, Select Project, etc.)
  for (let i = 0; i < lastLines.length; i++) {
    const line = lastLines[i].trim();
    if (line === "Resume Session" || line === "Select Project" ||
        line.match(/^(Resume|Select|Choose)\s+\w+$/)) {
      titleIndex = i;
      title = line;
      break;
    }
  }

  if (titleIndex === -1) return null;

  const items: SelectionMenuItem[] = [];
  let selectedIndex = 0;
  let instructions = "";

  // Parse items after title
  for (let i = titleIndex + 1; i < lastLines.length; i++) {
    const line = lastLines[i];
    const trimmed = line.trim();

    // Skip empty lines and search box
    if (!trimmed || trimmed.includes("Search")) continue;

    // Skip lines that are mostly box-drawing characters
    if (/^[─┌┐└┘│├┤┬┴┼╭╮╯╰\s>]+$/.test(trimmed)) continue;
    if (/^>\s*[╭╮╯╰─]+/.test(trimmed)) continue;

    // Skip lines that are just decorative borders
    const withoutBoxChars = trimmed.replace(/[─┌┐└┘│├┤┬┴┼╭╮╯╰>\s\.…]+/g, '');
    if (withoutBoxChars.length < 2) continue;

    // Check if this is an instruction line
    if (trimmed.includes("escape") || trimmed.includes("cancel") ||
        trimmed.match(/\s+to\s+\w+\s*·/) ||
        trimmed.includes("Type to search") ||
        trimmed.includes("to preview") ||
        trimmed.includes("to rename") ||
        trimmed.includes("to show all")) {
      instructions = trimmed;
      continue;
    }

    // Check if this looks like a session item (contains "ago" and "messages")
    if (trimmed.match(/\d+\s+(hour|day|week|month|year)s?\s+ago.*messages/i)) {
      // This is metadata - attach to previous item
      if (items.length > 0) {
        items[items.length - 1].description = trimmed;
      }
      continue;
    }

    // Check for selected item indicator
    const isSelected = line.includes("❯") || line.startsWith(">");

    // Extract label - remove selection indicators and box chars
    let label = trimmed
      .replace(/^[❯>▸▹\s]+/, '')
      .replace(/[╭╮╯╰─]+/g, '')
      .replace(/\(\+\d+\s+other\s+sessions?\)/i, '')
      .replace(/\.{2,}$/g, '')
      .trim();

    // Skip if empty after cleanup or looks like metadata or too short
    if (!label || label.length < 2) continue;
    if (label.match(/^\d+\s+(hour|day|week|month|year)s?\s+ago/i)) continue;

    if (isSelected) selectedIndex = items.length;

    items.push({
      label,
      isSelected,
    });
  }

  // Need at least 1 item to be a valid menu
  if (items.length < 1) return null;

  return {
    title,
    items,
    selectedIndex,
    hasSearch: true,
    instructions: instructions || undefined,
  };
}

// Detect if output contains an [AWAITING_INPUT] marker for brainstorming
// Format: [AWAITING_INPUT:taskId] or [AWAITING_INPUT]
export function detectAwaitingInput(buffer: string): AwaitingInputPrompt | null {
  const clean = stripAnsi(buffer);

  // Look for [AWAITING_INPUT] or [AWAITING_INPUT:123] pattern
  const markerRegex = /\[AWAITING_INPUT(?::(\d+))?\]/;
  const endMarker = "[/AWAITING_INPUT]";

  const match = clean.match(markerRegex);
  if (!match) return null;

  const startIndex = clean.lastIndexOf(match[0]);
  if (startIndex === -1) return null;

  const taskId = match[1] ? parseInt(match[1], 10) : undefined;

  // Check if there's content after the start marker
  const afterStart = clean.slice(startIndex + match[0].length);

  // If we find an end marker, the prompt is complete (user should answer)
  // If no end marker, prompt is still being output or waiting
  const endIndex = afterStart.indexOf(endMarker);

  // Extract the content between markers (or all content after start if no end marker yet)
  const content = endIndex !== -1
    ? afterStart.slice(0, endIndex).trim()
    : afterStart.trim();

  if (!content) return null;

  // Parse the content
  const lines = content.split("\n").map(l => l.trim()).filter(l => l);

  let question = "";
  const options: string[] = [];

  let inOptions = false;

  for (const line of lines) {
    if (line.startsWith("Question:")) {
      question = line.replace("Question:", "").trim();
    } else if (line === "Options:") {
      inOptions = true;
    } else if (inOptions && /^\d+\./.test(line)) {
      // Parse numbered option
      const optionText = line.replace(/^\d+\.\s*/, "").trim();
      if (optionText) {
        options.push(optionText);
      }
    } else if (!inOptions && !question) {
      // If no "Question:" prefix, treat first non-empty line as question
      question = line;
    }
  }

  if (!question) return null;

  return {
    taskId,
    question,
    options,
  };
}
