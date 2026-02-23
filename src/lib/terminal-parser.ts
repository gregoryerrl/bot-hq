import stripAnsi from "strip-ansi";

// Pre-process terminal buffer to preserve whitespace implied by cursor positioning.
// ANSI cursor movement sequences encode spatial layout - stripping them without
// preserving spacing causes words to concatenate ("BuildinteractiveCLI").
function preserveCursorSpacing(text: string): string {
  return text
    // Cursor to absolute column (e.g., \x1b[20G) → space
    .replace(/\x1b\[\d*G/g, ' ')
    // Cursor forward N positions (e.g., \x1b[5C) → space
    .replace(/\x1b\[\d*C/g, ' ')
    // Cursor position / home (e.g., \x1b[10;20H, \x1b[H) → newline
    .replace(/\x1b\[\d*;\d*[Hf]/g, '\n')
    .replace(/\x1b\[H/g, '\n')
    // Carriage return + line feed → newline
    .replace(/\r\n/g, '\n')
    // Standalone carriage return (cursor to start of line) → newline
    .replace(/\r/g, '\n');
}

// Additional cleanup for escape sequence fragments that strip-ansi misses
function cleanTerminalArtifacts(text: string): string {
  return text
    // Remove OSC sequences (like terminal title changes)
    .replace(/\x1b\][^\x07]*(?:\x07|\x1b\\)?/g, '')
    // Remove DCS sequences
    .replace(/\x1bP[^\x1b]*\x1b\\/g, '')
    // Remove mouse tracking and other application sequences
    .replace(/\x1b\[[\?<>=]?[0-9;]*[a-zA-Z]/g, '')
    // Remove partial escape fragments like *Mi, *sg, +sg, *un
    .replace(/[*+][A-Za-z][a-z]/g, '')
    // Remove cursor position reports and other CSI fragments
    .replace(/\x1b\[[0-9;]*[HfJKmsuABCDEFGnST]/g, '')
    // Remove single escape characters followed by brackets or letters
    .replace(/\x1b[\[\]()#][^\x1b]*/g, '')
    // Remove bare escape characters
    .replace(/\x1b/g, '')
    // Remove control characters except newline and tab
    .replace(/[\x00-\x08\x0B\x0C\x0E-\x1F\x7F]/g, '')
    // Clean up multiple spaces
    .replace(/[ ]{3,}/g, '  ')
    // Clean up multiple dots that aren't ellipsis
    .replace(/\.{4,}/g, '...')
    // Remove lines that are just dots/periods
    .replace(/^[\.\s]+$/gm, '')
    // Clean up multiple newlines
    .replace(/\n{3,}/g, '\n\n');
}

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

  // Claude Code session rating prompt
  /How is Claude doing this session/i,
  /^\s*\d:\s*(Bad|Fine|Good|Dismiss)\s*/i,
  /^1:\s*Bad\s+2:\s*Fine\s+3:\s*Good/i,

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

  // Skip very short lines (likely UI artifacts or streaming fragments)
  if (trimmed.length < 4) return true;

  // Skip garbled escape sequences
  if (/\^\[\[/.test(trimmed)) return true;
  if (/\[\[O/.test(trimmed)) return true;
  if (/^[a-z]\^/.test(trimmed)) return true;

  // Skip partial escape sequence fragments (e.g., *Mi, *sg, +sg, *un)
  if (/^[*+][A-Za-z][a-z]$/.test(trimmed)) return true;
  if (/^[*+][A-Za-z][a-z]\s/.test(trimmed)) return true;

  // Skip lines that are just ellipsis or dots
  if (/^\.{2,}$/.test(trimmed)) return true;
  if (/^…+$/.test(trimmed)) return true;

  // Skip lines that are just code block markers
  if (/^`{3,}$/.test(trimmed)) return true;
  if (/^`{3,}\w*$/.test(trimmed)) return true;

  // Skip fragmented table lines (single column separators)
  if (/^[│|┃]+$/.test(trimmed)) return true;

  // Skip lines that look like cursor/screen control remnants
  if (/^\[\d+[A-Z]$/.test(trimmed)) return true;
  if (/^\d+;\d+[Hf]$/.test(trimmed)) return true;

  // Skip lines that are just prompt characters
  if (/^[❯>●]\s*$/.test(trimmed)) return true;

  // Skip prefix markers (* + · ●) followed by very short content (streaming fragments)
  // e.g., "* S", "+ e", "· K ↑", "* 4", "+ in ..."
  if (/^[*+·●]\s*.{0,8}$/.test(trimmed)) return true;

  // Skip Claude Code thinking/streaming progress indicators
  // These are in-place updated status lines that fragment when parsed
  if (/\(thinking\)\s*$/i.test(trimmed)) return true;
  if (/^[*+·]\s*\w{0,3}\s*\(thinking\)/i.test(trimmed)) return true;
  if (/^[*+·]\s*(Stewing|Propagating|Photosynthesizing|Gitifying|Lollygagging|Ruminating|Pondering|Cogitating|Musing|Deliberating|Percolating|Simmering|Marinating|Distilling|Crystallizing|Composting|Fermenting|Brainstorming|Noodling|Daydreaming|Meditating|Contemplating|Reflecting|Processing|Synthesizing|Assembling|Constructing|Formulating|Drafting|Sculpting|Weaving|Brewing|Baking|Cooking|Sauteing|Grilling|Roasting|Mixing|Blending|Stirring|Whipping|Folding|Kneading|Crunching|Churning|Grinding|Polishing|Buffing|Sanding|Hammering|Forging|Welding|Soldering|Wiring|Plumbing|Tinkering|Fiddling|Juggling|Puzzling|Untangling|Decoding|Parsing|Compiling|Linking|Deploying|Shipping|Launching|Loading|Rendering|Painting|Sketching|Doodling|Scribbling|Typing|Writing|Reading|Scanning|Analyzing|Computing|Calculating|Tabulating|Enumerating|Iterating|Recursing|Searching|Indexing|Caching|Buffering|Streaming|Piping|Routing|Mapping|Reducing|Filtering|Sorting|Hashing|Encrypting|Decrypting|Compressing|Decompressing)/i.test(trimmed)) return true;
  if (/^\d+m?\s*\d*s?\s*·\s*[↓↑]\s*[\d.]+k?\s*tokens/i.test(trimmed)) return true;
  if (/·\s*[↓↑]\s*[\d.]+k?\s*tokens\s*·?\s*(thinking)?/i.test(trimmed)) return true;
  // Streaming timing results: "Crunched for 1m 26s", "Kneading... 6"
  if (/^(Crunched|Kneaded|Stewed|Brewed|Baked|Cooked)\s+(for\s+)?\d/i.test(trimmed)) return true;
  if (/^[A-Z][a-z]+ing\.{3}\s*[↓↑]?\s*\d*$/i.test(trimmed)) return true;
  // Single or few characters followed by (thinking) - fragmented progress updates
  if (/^[*+·]?\s*[A-Za-z\s]{0,5}\s*\d*\s*(thinking)?\s*$/i.test(trimmed) && trimmed.length < 20) return true;
  // "Claude is working..." spinner text
  if (/^Claude is working/i.test(trimmed)) return true;
  // ctrl+o/ctrl+b hints
  if (/ctrl\+[ob]\s+to\s+(expand|run)/i.test(trimmed)) return true;
  // "+N more tool uses" lines
  if (/^\+\d+\s+more\s+tool\s+uses/i.test(trimmed)) return true;
  // "[Pasted text #N +N lines]" indicators
  if (/^\[Pasted text/i.test(trimmed)) return true;
  // "bypass permissions on/off" status bar
  if (/bypass permissions/i.test(trimmed)) return true;
  // "shift+tab to cycle" and similar status bar text
  if (/shift\+tab\s+to\s+cycle/i.test(trimmed)) return true;

  // Skip lines that are mostly box-drawing or decorative
  if (/^[─┌┐└┘│├┤┬┴┼╭╮╯╰\s|└├│┃┆┊]+$/.test(trimmed)) return true;

  // Skip lines that start with box-drawing chars (tool output borders, tree connectors)
  if (/^[└├│┃┆┊╭╮╯╰─┌┐┘]/.test(trimmed)) return true;

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
  // Preserve cursor-implied spacing, strip ANSI codes, then clean artifacts
  const clean = cleanTerminalArtifacts(stripAnsi(preserveCursorSpacing(buffer)));
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
    /^[*+][A-Za-z][a-z]/,              // Partial escape sequences
    /^\.{2,}$/,                         // Just dots/ellipsis
    /^…+$/,                             // Unicode ellipsis
    /^\[\d+[A-Z]/,                      // Cursor control remnants
    /^`{3,}$/,                          // Code block markers only
    /\(thinking\)/i,                    // Thinking indicators
    /^[*+·]\s*(Stewing|Propagating|Photosynthesizing|Gitifying|Lollygagging|Ruminating|Pondering|Kneading|Crunching|Churning)/i,
    /·\s*[↓↑]\s*[\d.]+k?\s*tokens/i,   // Token count lines
    /^Claude is working/i,              // Working spinner
    /ctrl\+[ob]\s+to\s+(expand|run)/i,  // ctrl+o/b hints
    /^\+\d+\s+more\s+tool\s+uses/i,    // Tool use count
    /How is Claude doing this session/i, // Session rating
    /^\d:\s*(Bad|Fine|Good|Dismiss)/i,  // Rating options
    /^\[Pasted text/i,                  // Pasted text indicators
    /bypass permissions/i,              // Status bar
    /shift\+tab\s+to\s+cycle/i,        // Status bar
    /^(Crunched|Kneaded|Stewed|Brewed)\s+(for\s+)?\d/i, // Timing results
    /^[*+·●]\s*.{0,8}$/,               // Short streaming fragments
    /^[└├│┃┆┊╭╮╯╰─┌┐┘]/,              // Box-drawing prefixes
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

  // First check for explicit [AWAITING_INPUT] marker
  const markerRegex = /\[AWAITING_INPUT(?::(\d+))?\]/;
  const endMarker = "[/AWAITING_INPUT]";

  const match = clean.match(markerRegex);
  if (match) {
    const startIndex = clean.lastIndexOf(match[0]);
    if (startIndex !== -1) {
      const taskId = match[1] ? parseInt(match[1], 10) : undefined;
      const afterStart = clean.slice(startIndex + match[0].length);
      const endIndex = afterStart.indexOf(endMarker);
      const content = endIndex !== -1
        ? afterStart.slice(0, endIndex).trim()
        : afterStart.trim();

      if (content) {
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
            const optionText = line.replace(/^\d+\.\s*/, "").trim();
            if (optionText) options.push(optionText);
          } else if (!inOptions && !question) {
            question = line;
          }
        }

        if (question) {
          return { taskId, question, options };
        }
      }
    }
  }

  // Also detect natural question patterns from Claude Code
  // Only look at the LAST 15 lines to avoid matching prompt template text
  // that's deeper in the buffer (e.g., example "[AWAITING_INPUT]" blocks)
  const lines = clean.split("\n").filter(l => l.trim());
  const lastLines = lines.slice(-15);

  // Patterns that indicate Claude is waiting for user input
  // These must be actual questions, not status messages
  const questionPatterns = [
    // Direct questions (these inherently indicate awaiting input)
    /what would you like/i,
    /would you like (?:me )?to/i,
    /want me to (?:create|start|do|help|make)/i,
    /please (?:select|choose|pick|tell|let)/i,
    /which (?:option|one|workspace|task|would)/i,
    /how would you like/i,
    /should i (?:proceed|continue|start|create)/i,
    /do you want (?:me )?to/i,
    /what should i do/i,
    /how can i help/i,
    /can i help/i,
    // Instruction prompts that expect input
    /just tell me/i,
    /tell me[:\s]/i,
    /let me know/i,
    /specify (?:which|what|the|a)/i,
    /provide (?:the|a|your|me)/i,
    // Question marks at end of line (clear questions)
    /\?$/,
  ];

  // Patterns that look like status/waiting messages and should NOT trigger input detection
  const statusPatterns = [
    /waiting for task command/i,
    /ready\.\s*waiting/i,
    /waiting for task\s*$/i,
    /ready for next/i,
    /awaiting task/i,
    /how is claude doing this session/i,  // Session rating prompt
    /^\d+:\s*(Bad|Fine|Good|Dismiss)/i,   // Session rating options
    /1:\s*Bad\s+2:\s*Fine\s+3:\s*Good/i,  // Session rating line
    /\(optional\)\s*$/i,                   // "(optional)" suffix
    /Question:\s*Your question here/i,     // Template example text
    /accept.*commit.*merge.*retry.*discard/i, // Manager review question (should not trigger)
    /Stopped\.\s*What\s*would\s*you/i,    // Claude Code stop prompt (handled by manager)
    /First option/i,                       // Template example option
    /Third option/i,                       // Template example option
  ];

  // Look for question pattern in the last lines
  let questionLineIndex = -1;
  let questionText = "";

  for (let i = lastLines.length - 1; i >= 0; i--) {
    const line = lastLines[i].trim();

    // Skip lines that end with "." (declarative statements, not questions)
    if (line.endsWith('.') && !line.endsWith('?')) continue;

    // Skip known status/waiting messages
    if (statusPatterns.some(p => p.test(line))) continue;

    // Check if this line matches a question pattern
    for (const pattern of questionPatterns) {
      if (pattern.test(line)) {
        questionLineIndex = i;
        questionText = line;
        break;
      }
    }
    if (questionLineIndex !== -1) break;
  }

  // If we found a question, look for options nearby
  if (questionLineIndex !== -1) {
    const options: string[] = [];

    // Look backwards from the question for numbered options
    for (let i = questionLineIndex - 1; i >= Math.max(0, questionLineIndex - 15); i--) {
      const line = lastLines[i].trim();
      const optionMatch = line.match(/^(\d+)\.\s+(.+)/);
      if (optionMatch) {
        const optionText = optionMatch[2].trim();
        // Skip instruction-like lines
        if (optionText && !optionText.match(/^(Esc|Tab|Enter|ctrl)/i)) {
          options.unshift(optionText); // Add to beginning to maintain order
        }
      }
    }

    // Also look forward for options (in case question comes before options)
    for (let i = questionLineIndex + 1; i < Math.min(lastLines.length, questionLineIndex + 10); i++) {
      const line = lastLines[i].trim();
      const optionMatch = line.match(/^(\d+)\.\s+(.+)/);
      if (optionMatch) {
        const optionText = optionMatch[2].trim();
        if (optionText && !optionText.match(/^(Esc|Tab|Enter|ctrl)/i)) {
          if (!options.includes(optionText)) {
            options.push(optionText);
          }
        }
      }
    }

    // Return even without options if we found a clear question
    return {
      question: questionText,
      options,
    };
  }

  // Also check for "Options:" style lists at the end
  const optionsIndex = lastLines.findIndex(l => l.trim() === "Options:");
  if (optionsIndex !== -1) {
    const options: string[] = [];
    let question = "";

    // Look for question before "Options:"
    for (let i = optionsIndex - 1; i >= Math.max(0, optionsIndex - 5); i--) {
      const line = lastLines[i].trim();
      if (line && !line.match(/^[-─═]+$/) && line.length > 10) {
        question = line;
        break;
      }
    }

    // Parse options after "Options:"
    for (let i = optionsIndex + 1; i < Math.min(lastLines.length, optionsIndex + 10); i++) {
      const line = lastLines[i].trim();
      const optionMatch = line.match(/^(\d+)\.\s+(.+)/);
      if (optionMatch) {
        const optionText = optionMatch[2].trim();
        if (optionText) options.push(optionText);
      }
    }

    if (options.length > 0) {
      return { question: question || "Please select an option", options };
    }
  }

  // Check for numbered options at the very end (common pattern)
  // Only trigger if there's a clear question (ending with ?) - NOT colon
  // Colon-ending lines are often instructions/headers, not questions
  const lastFewLines = lastLines.slice(-15);
  const numberedOptions: string[] = [];
  let lastQuestionLine = "";

  for (const line of lastFewLines) {
    const trimmed = line.trim();

    // Skip known status patterns
    if (statusPatterns.some(p => p.test(trimmed))) continue;

    const optionMatch = trimmed.match(/^(\d+)\.\s+(.+)/);
    if (optionMatch) {
      const optionText = optionMatch[2].trim();
      if (optionText && !optionText.match(/^(Esc|Tab|Enter|ctrl)/i) && optionText.length > 3) {
        numberedOptions.push(optionText);
      }
    } else if (trimmed.endsWith("?")) {
      // Only treat lines ending with ? as questions (not :)
      lastQuestionLine = trimmed;
    }
  }

  if (numberedOptions.length >= 2 && lastQuestionLine) {
    return {
      question: lastQuestionLine,
      options: numberedOptions,
    };
  }

  return null;
}
