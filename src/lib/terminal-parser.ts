import stripAnsi from "strip-ansi";

export type ParsedBlock =
  | { type: "assistant"; content: string }
  | { type: "user"; content: string }
  | { type: "code"; content: string; lang?: string }
  | { type: "tool"; name: string; output: string }
  | { type: "permission"; question: string; options: string[] }
  | { type: "thinking"; content: string };

export interface PermissionPrompt {
  question: string;
  options: string[];
  selectedIndex: number;
}

// Detect if output ends with a permission prompt
export function detectPermissionPrompt(buffer: string): PermissionPrompt | null {
  const clean = stripAnsi(buffer);
  const lines = clean.split("\n").filter((l) => l.trim());

  // Look for pattern: ? question followed by numbered options with ❯ indicator
  // Find the last question line
  let questionIndex = -1;
  for (let i = lines.length - 1; i >= 0; i--) {
    if (lines[i].trim().startsWith("?")) {
      questionIndex = i;
      break;
    }
  }

  if (questionIndex === -1) return null;

  const question = lines[questionIndex].replace(/^\?\s*/, "").trim();
  const options: string[] = [];
  let selectedIndex = 0;

  // Parse options after the question
  for (let i = questionIndex + 1; i < lines.length; i++) {
    const line = lines[i].trim();
    // Match patterns like "❯ 1. Yes" or "  2. No" or "> Yes"
    const optionMatch = line.match(/^([❯>]\s*)?(\d+\.\s*)?(.+)$/);
    if (optionMatch) {
      const isSelected = line.startsWith("❯") || line.startsWith(">");
      const optionText = optionMatch[3].trim();
      if (optionText && !optionText.startsWith("?")) {
        if (isSelected) selectedIndex = options.length;
        options.push(optionText);
      }
    }
  }

  if (options.length < 2) return null;

  return { question, options, selectedIndex };
}

// Parse full buffer into blocks for chat view
export function parseTerminalOutput(buffer: string): ParsedBlock[] {
  const clean = stripAnsi(buffer);
  const blocks: ParsedBlock[] = [];

  // Split by common delimiters
  const lines = clean.split("\n");
  let currentBlock: string[] = [];
  let currentType: ParsedBlock["type"] = "assistant";

  for (const line of lines) {
    // Detect user input (lines starting with > or after prompt)
    if (line.match(/^>\s/) || line.match(/^❯\s*\d+\./)) {
      // Flush current block
      if (currentBlock.length > 0) {
        blocks.push({ type: currentType, content: currentBlock.join("\n").trim() });
        currentBlock = [];
      }
      continue;
    }

    // Detect code blocks (lines with consistent indentation or ````)
    if (line.startsWith("```") || line.match(/^\s{4,}/)) {
      if (currentType !== "code" && currentBlock.length > 0) {
        blocks.push({ type: currentType, content: currentBlock.join("\n").trim() });
        currentBlock = [];
      }
      currentType = "code";
    }

    // Detect tool output (common patterns)
    if (line.match(/^(Read|Write|Edit|Bash|Glob|Grep):/i)) {
      if (currentBlock.length > 0) {
        blocks.push({ type: currentType, content: currentBlock.join("\n").trim() });
        currentBlock = [];
      }
      currentType = "tool";
    }

    currentBlock.push(line);
  }

  // Flush remaining
  if (currentBlock.length > 0) {
    const content = currentBlock.join("\n").trim();
    if (content) {
      blocks.push({ type: currentType, content });
    }
  }

  return blocks.filter((b) => b.type === "assistant" || b.type === "code" || b.type === "tool" ? b.content.length > 0 : true);
}

// Check if the "tell claude" option is selected
export function isTellClaudeSelected(prompt: PermissionPrompt): boolean {
  const selected = prompt.options[prompt.selectedIndex]?.toLowerCase() || "";
  return selected.includes("tell claude") || selected.includes("do differently") || selected.includes("feedback");
}
