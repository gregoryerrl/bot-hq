import { db, prompts } from "@/lib/db";
import { eq } from "drizzle-orm";
import {
  getDefaultManagerPrompt,
  getSWEngineerPrompt,
  getReInitPrompt,
  getAssistantManagerStartupPrompt,
  buildSWEngineerTemplate,
} from "@/lib/bot-hq/templates";
import type { Prompt } from "@/lib/db/schema";
import fs from "fs/promises";
import path from "path";

export const PROMPT_SLUGS = {
  MANAGER: "manager",
  SW_ENGINEER: "sw-engineer",
  REINIT: "reinit",
  STARTUP_AUDIT: "startup-audit",
} as const;

type PromptSlug = (typeof PROMPT_SLUGS)[keyof typeof PROMPT_SLUGS];

interface DefaultPromptDef {
  slug: PromptSlug;
  name: string;
  description: string;
  content: string;
  variables: string[] | null;
  isParametric: boolean;
}

function getDefaultPromptDefs(): DefaultPromptDef[] {
  return [
    {
      slug: PROMPT_SLUGS.MANAGER,
      name: "Manager Prompt",
      description:
        "Main orchestrator prompt. Controls task flow, subagent spawning, and system startup.",
      content: getDefaultManagerPrompt(),
      variables: null,
      isParametric: false,
    },
    {
      slug: PROMPT_SLUGS.SW_ENGINEER,
      name: "SW Engineer Prompt",
      description:
        "Template for the software engineer subagent. Uses mustache variables for task-specific values.",
      content: buildSWEngineerTemplate(),
      variables: [
        "taskId",
        "title",
        "description",
        "repoPath",
        "workspaceName",
        "feedbackBlock",
        "branchBlock",
        "branchName",
        "slug",
      ],
      isParametric: true,
    },
    {
      slug: PROMPT_SLUGS.REINIT,
      name: "ReInit Prompt",
      description:
        "Sent after self-clear to re-initialize the manager with minimal context.",
      content: getReInitPrompt(),
      variables: null,
      isParametric: false,
    },
    {
      slug: PROMPT_SLUGS.STARTUP_AUDIT,
      name: "Startup Audit Prompt",
      description:
        "Assistant Manager Bot startup mode prompt. Embedded in the Manager prompt for system health audits.",
      content: getAssistantManagerStartupPrompt(),
      variables: null,
      isParametric: false,
    },
  ];
}

/**
 * Seed default prompts into the DB if empty.
 * Migration path: if .bot-hq/MANAGER_PROMPT.md exists with custom content, use that.
 */
export async function seedDefaultPrompts(): Promise<void> {
  const existing = await db.select().from(prompts).limit(1);
  if (existing.length > 0) return;

  const defaults = getDefaultPromptDefs();

  // Check for custom manager prompt on filesystem
  const botHqRoot = process.env.BOT_HQ_SCOPE || "/Users/gregoryerrl/Projects";
  const managerPromptPath = path.join(botHqRoot, ".bot-hq", "MANAGER_PROMPT.md");
  try {
    const customContent = await fs.readFile(managerPromptPath, "utf-8");
    const defaultContent = getDefaultManagerPrompt();
    if (customContent.trim() !== defaultContent.trim()) {
      const managerDef = defaults.find((d) => d.slug === PROMPT_SLUGS.MANAGER);
      if (managerDef) {
        managerDef.content = customContent;
      }
    }
  } catch {
    // File doesn't exist — use hardcoded default
  }

  for (const def of defaults) {
    await db.insert(prompts).values({
      slug: def.slug,
      name: def.name,
      description: def.description,
      content: def.content,
      variables: def.variables ? JSON.stringify(def.variables) : null,
      isParametric: def.isParametric,
    });
  }
}

/**
 * Get a prompt by slug, falling back to hardcoded default.
 */
export async function getPromptBySlug(slug: string): Promise<string> {
  try {
    const result = await db
      .select()
      .from(prompts)
      .where(eq(prompts.slug, slug))
      .limit(1);

    if (result.length > 0) return result[0].content;
  } catch {
    // DB not ready — fall back
  }

  // Fallback to hardcoded defaults
  switch (slug) {
    case PROMPT_SLUGS.MANAGER:
      return getDefaultManagerPrompt();
    case PROMPT_SLUGS.SW_ENGINEER:
      return buildSWEngineerTemplate();
    case PROMPT_SLUGS.REINIT:
      return getReInitPrompt();
    case PROMPT_SLUGS.STARTUP_AUDIT:
      return getAssistantManagerStartupPrompt();
    default:
      throw new Error(`Unknown prompt slug: ${slug}`);
  }
}

/**
 * Get full prompt record by slug.
 */
export async function getPromptRecordBySlug(
  slug: string
): Promise<Prompt | null> {
  const result = await db
    .select()
    .from(prompts)
    .where(eq(prompts.slug, slug))
    .limit(1);

  return result.length > 0 ? result[0] : null;
}

/**
 * Get all prompts for the list UI.
 */
export async function getAllPrompts(): Promise<Prompt[]> {
  return db.select().from(prompts);
}

/**
 * Update a prompt's content.
 */
export async function updatePromptContent(
  slug: string,
  content: string
): Promise<Prompt> {
  const result = await db
    .update(prompts)
    .set({ content, updatedAt: new Date() })
    .where(eq(prompts.slug, slug))
    .returning();

  if (result.length === 0) {
    throw new Error(`Prompt not found: ${slug}`);
  }
  return result[0];
}

/**
 * Reset a prompt to its hardcoded default.
 */
export async function resetPromptToDefault(slug: string): Promise<Prompt> {
  const defaults = getDefaultPromptDefs();
  const def = defaults.find((d) => d.slug === slug);
  if (!def) throw new Error(`Unknown prompt slug: ${slug}`);

  return updatePromptContent(slug, def.content);
}

/**
 * Replace {{variable}} placeholders in a template string.
 */
export function interpolatePrompt(
  template: string,
  vars: Record<string, string>
): string {
  return template.replace(/\{\{(\w+)\}\}/g, (match, key) => {
    return key in vars ? vars[key] : match;
  });
}

/**
 * Build a fully interpolated SW Engineer prompt from the DB template.
 */
export async function buildSWEngineerFromDB(params: {
  taskId: number;
  title: string;
  description: string;
  repoPath: string;
  workspaceName: string;
  iterationCount: number;
  feedback?: string | null;
  existingBranch?: string | null;
}): Promise<string> {
  let template: string;
  try {
    template = await getPromptBySlug(PROMPT_SLUGS.SW_ENGINEER);
  } catch {
    // Ultimate fallback: use the old function
    return getSWEngineerPrompt(params);
  }

  const slug = params.title
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 30);

  const branchName =
    params.existingBranch || `task/${params.taskId}-${slug}`;

  const feedbackBlock = params.feedback
    ? `## Previous Feedback (Iteration ${params.iterationCount})\n${params.feedback}\n\nPlease address this feedback in your implementation.\n`
    : "";

  const branchBlock = params.existingBranch
    ? `Switch to Existing Branch\n\`\`\`bash\ncd ${params.repoPath}\ngit stash --include-untracked 2>/dev/null || true\ngit checkout ${branchName}\n\`\`\``
    : `Create Feature Branch\n\`\`\`bash\ncd ${params.repoPath}\ngit stash --include-untracked 2>/dev/null || true\ngit checkout main\ngit pull origin main 2>/dev/null || true\ngit checkout -b ${branchName}\n\`\`\``;

  return interpolatePrompt(template, {
    taskId: String(params.taskId),
    title: params.title,
    description: params.description,
    repoPath: params.repoPath,
    workspaceName: params.workspaceName,
    feedbackBlock,
    branchBlock,
    branchName,
    slug,
  });
}
