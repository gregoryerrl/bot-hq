// src/app/api/plugins/actions/execute/route.ts

import { NextRequest, NextResponse } from "next/server";
import { getPluginEvents, getPluginRegistry, createPluginContext } from "@/lib/plugins";
import { db, tasks, approvals, workspaces } from "@/lib/db";
import { eq } from "drizzle-orm";

// Action execution timeout (30 seconds)
const ACTION_TIMEOUT_MS = 30000;

interface ErrorResponse {
  error: string;
  code: string;
  details?: string;
}

function errorResponse(
  error: string,
  code: string,
  status: number,
  details?: string
): NextResponse<ErrorResponse> {
  return NextResponse.json({ error, code, details }, { status });
}

// Execute with timeout wrapper
async function executeWithTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  actionName: string
): Promise<T> {
  let timeoutId: NodeJS.Timeout;
  const timeoutPromise = new Promise<never>((_, reject) => {
    timeoutId = setTimeout(() => {
      reject(new Error(`Action "${actionName}" timed out after ${timeoutMs}ms`));
    }, timeoutMs);
  });

  try {
    const result = await Promise.race([promise, timeoutPromise]);
    clearTimeout(timeoutId!);
    return result;
  } catch (err) {
    clearTimeout(timeoutId!);
    throw err;
  }
}

export async function POST(request: NextRequest) {
  let pluginName: string | undefined;
  let actionId: string | undefined;

  try {
    // Parse request body
    let body: Record<string, unknown>;
    try {
      body = await request.json();
    } catch {
      return errorResponse(
        "Invalid JSON in request body",
        "INVALID_REQUEST_BODY",
        400
      );
    }

    pluginName = body.pluginName as string | undefined;
    actionId = body.actionId as string | undefined;
    const approvalId = body.approvalId as number | undefined;

    // Validate required fields with specific messages
    if (!pluginName) {
      return errorResponse(
        "Missing required field: pluginName",
        "MISSING_PLUGIN_NAME",
        400
      );
    }
    if (!actionId) {
      return errorResponse(
        "Missing required field: actionId",
        "MISSING_ACTION_ID",
        400
      );
    }
    if (approvalId === undefined) {
      return errorResponse(
        "Missing required field: approvalId",
        "MISSING_APPROVAL_ID",
        400
      );
    }

    // Get approval details
    const approval = await db.query.approvals.findFirst({
      where: eq(approvals.id, approvalId),
    });

    if (!approval) {
      return errorResponse(
        `Approval with ID ${approvalId} not found`,
        "APPROVAL_NOT_FOUND",
        404
      );
    }

    // Get task and workspace
    const task = await db.query.tasks.findFirst({
      where: eq(tasks.id, approval.taskId),
    });

    if (!task) {
      return errorResponse(
        `Task with ID ${approval.taskId} not found for approval ${approvalId}`,
        "TASK_NOT_FOUND",
        404
      );
    }

    const workspace = await db.query.workspaces.findFirst({
      where: eq(workspaces.id, approval.workspaceId),
    });

    if (!workspace) {
      return errorResponse(
        `Workspace with ID ${approval.workspaceId} not found for approval ${approvalId}`,
        "WORKSPACE_NOT_FOUND",
        404
      );
    }

    // Get plugin and its actions
    const registry = getPluginRegistry();
    const plugin = registry.getPlugin(pluginName);

    if (!plugin) {
      return errorResponse(
        `Plugin "${pluginName}" not found. Is the plugin installed?`,
        "PLUGIN_NOT_FOUND",
        404
      );
    }

    if (!plugin.enabled) {
      return errorResponse(
        `Plugin "${pluginName}" is installed but disabled. Enable it in the plugins page.`,
        "PLUGIN_DISABLED",
        400
      );
    }

    // Find the action
    const events = getPluginEvents();
    let approvalActions;
    try {
      approvalActions = await events.getApprovalActions();
    } catch (err) {
      return errorResponse(
        "Failed to retrieve plugin actions",
        "ACTIONS_RETRIEVAL_ERROR",
        500,
        err instanceof Error ? err.message : "Unknown error"
      );
    }

    const actionDef = approvalActions.find(
      a => a.pluginName === pluginName && a.action.id === actionId
    );

    if (!actionDef) {
      return errorResponse(
        `Action "${actionId}" not found in plugin "${pluginName}"`,
        "ACTION_NOT_FOUND",
        404,
        `Available actions: ${approvalActions
          .filter(a => a.pluginName === pluginName)
          .map(a => a.action.id)
          .join(", ") || "none"}`
      );
    }

    // Create context
    let context;
    try {
      context = await createPluginContext(plugin);
    } catch (err) {
      return errorResponse(
        `Failed to create plugin context for "${pluginName}"`,
        "CONTEXT_CREATION_ERROR",
        500,
        err instanceof Error ? err.message : "Unknown error"
      );
    }

    // Parse stored JSON data safely
    let commitMessages: string[] = [];
    let diffSummary: unknown = null;
    try {
      if (approval.commitMessages) {
        commitMessages = JSON.parse(approval.commitMessages);
      }
      if (approval.diffSummary) {
        diffSummary = JSON.parse(approval.diffSummary);
      }
    } catch {
      console.warn(`Failed to parse approval data for approval ${approvalId}`);
    }

    const actionContext = {
      approval: {
        id: approval.id,
        branchName: approval.branchName,
        baseBranch: approval.baseBranch,
        commitMessages,
        diffSummary,
      },
      task: {
        id: task.id,
        title: task.title,
        description: task.description || "",
        state: task.state,
      },
      workspace: {
        id: workspace.id,
        name: workspace.name,
        repoPath: workspace.repoPath,
      },
      pluginContext: context,
    };

    // Execute action with timeout
    let result;
    try {
      result = await executeWithTimeout(
        actionDef.action.handler(actionContext),
        ACTION_TIMEOUT_MS,
        actionId
      );
    } catch (err) {
      const message = err instanceof Error ? err.message : "Unknown execution error";
      const isTimeout = message.includes("timed out");

      return errorResponse(
        `Action "${actionId}" execution failed: ${message}`,
        isTimeout ? "ACTION_TIMEOUT" : "ACTION_EXECUTION_ERROR",
        isTimeout ? 504 : 500,
        `Plugin: ${pluginName}, Approval: ${approvalId}`
      );
    }

    // Validate result structure
    if (typeof result !== "object" || result === null) {
      return errorResponse(
        `Action "${actionId}" returned invalid result`,
        "INVALID_ACTION_RESULT",
        500,
        "Handler must return an object with success, message, and optional data"
      );
    }

    return NextResponse.json({
      success: result.success,
      message: result.message,
      data: result.data,
    });
  } catch (error) {
    // Catch any unexpected errors
    const message = error instanceof Error ? error.message : "Unknown error";
    console.error("Unexpected error in plugin action execution:", {
      pluginName,
      actionId,
      error: message,
      stack: error instanceof Error ? error.stack : undefined,
    });

    return errorResponse(
      "An unexpected error occurred while executing the action",
      "UNEXPECTED_ERROR",
      500,
      message
    );
  }
}
