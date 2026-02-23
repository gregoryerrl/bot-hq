import fs from "fs/promises";
import path from "path";
import { getDefaultManagerPrompt, getDefaultWorkspaceTemplate } from "./templates";

const BOT_HQ_ROOT = process.env.BOT_HQ_SCOPE || "/Users/gregoryerrl/Projects";
const BOT_HQ_DIR = path.join(BOT_HQ_ROOT, ".bot-hq");

export async function initializeBotHqStructure(): Promise<void> {
  // Create main .bot-hq directory
  await fs.mkdir(BOT_HQ_DIR, { recursive: true });
  await fs.mkdir(path.join(BOT_HQ_DIR, "workspaces"), { recursive: true });

  // Create MANAGER_PROMPT.md if it doesn't exist
  const managerPromptPath = path.join(BOT_HQ_DIR, "MANAGER_PROMPT.md");
  try {
    await fs.access(managerPromptPath);
  } catch {
    await fs.writeFile(managerPromptPath, getDefaultManagerPrompt());
  }

  // Create QUEUE.md if it doesn't exist
  const queuePath = path.join(BOT_HQ_DIR, "QUEUE.md");
  try {
    await fs.access(queuePath);
  } catch {
    await fs.writeFile(queuePath, "# Task Queue\n\nNo tasks currently running.\n");
  }
}

export async function initializeWorkspaceContext(workspaceName: string): Promise<void> {
  const workspaceDir = path.join(BOT_HQ_DIR, "workspaces", workspaceName);
  await fs.mkdir(workspaceDir, { recursive: true });
  await fs.mkdir(path.join(workspaceDir, "tasks"), { recursive: true });

  const workspaceMdPath = path.join(workspaceDir, "WORKSPACE.md");
  try {
    await fs.access(workspaceMdPath);
  } catch {
    await fs.writeFile(workspaceMdPath, getDefaultWorkspaceTemplate(workspaceName));
  }

  const stateMdPath = path.join(workspaceDir, "STATE.md");
  try {
    await fs.access(stateMdPath);
  } catch {
    await fs.writeFile(stateMdPath, "# Current State\n\nNo active state.\n");
  }
}

export async function getManagerPrompt(): Promise<string> {
  const promptPath = path.join(BOT_HQ_DIR, "MANAGER_PROMPT.md");
  try {
    return await fs.readFile(promptPath, "utf-8");
  } catch {
    return getDefaultManagerPrompt();
  }
}

export async function saveManagerPrompt(content: string): Promise<void> {
  const promptPath = path.join(BOT_HQ_DIR, "MANAGER_PROMPT.md");
  await fs.writeFile(promptPath, content);
}

export async function getWorkspaceContext(workspaceName: string): Promise<string> {
  const workspaceMdPath = path.join(BOT_HQ_DIR, "workspaces", workspaceName, "WORKSPACE.md");
  try {
    return await fs.readFile(workspaceMdPath, "utf-8");
  } catch {
    return "";
  }
}

export async function saveWorkspaceContext(workspaceName: string, content: string): Promise<void> {
  const workspaceDir = path.join(BOT_HQ_DIR, "workspaces", workspaceName);
  await fs.mkdir(workspaceDir, { recursive: true });
  await fs.writeFile(path.join(workspaceDir, "WORKSPACE.md"), content);
}

export async function getTaskProgress(workspaceName: string, taskId: number): Promise<string | null> {
  const progressPath = path.join(BOT_HQ_DIR, "workspaces", workspaceName, "tasks", String(taskId), "PROGRESS.md");
  try {
    return await fs.readFile(progressPath, "utf-8");
  } catch {
    return null;
  }
}

export async function cleanupTaskFiles(workspaceName: string, taskId: number): Promise<void> {
  const taskDir = path.join(BOT_HQ_DIR, "workspaces", workspaceName, "tasks", String(taskId));
  try {
    await fs.rm(taskDir, { recursive: true });
  } catch {
    // Directory may not exist
  }
}

export async function clearAllTaskContext(workspaceName: string): Promise<void> {
  const tasksDir = path.join(BOT_HQ_DIR, "workspaces", workspaceName, "tasks");
  try {
    await fs.rm(tasksDir, { recursive: true });
    await fs.mkdir(tasksDir, { recursive: true });
  } catch {
    // Directory may not exist
  }

  // Reset STATE.md
  await updateStateFile(workspaceName, "# Current State\n\nNo active state.\n");
}

export async function updateStateFile(workspaceName: string, content: string): Promise<void> {
  const stateMdPath = path.join(BOT_HQ_DIR, "workspaces", workspaceName, "STATE.md");
  const workspaceDir = path.join(BOT_HQ_DIR, "workspaces", workspaceName);
  await fs.mkdir(workspaceDir, { recursive: true });
  await fs.writeFile(stateMdPath, content);
}

export { BOT_HQ_DIR, BOT_HQ_ROOT };
