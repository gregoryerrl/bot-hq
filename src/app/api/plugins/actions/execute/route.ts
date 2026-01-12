// src/app/api/plugins/actions/execute/route.ts

import { NextRequest, NextResponse } from "next/server";
import { getPluginEvents, getPluginRegistry, createPluginContext } from "@/lib/plugins";
import { db, tasks, approvals, workspaces } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function POST(request: NextRequest) {
  try {
    const body = await request.json();
    const { pluginName, actionId, approvalId } = body;

    if (!pluginName || !actionId || !approvalId) {
      return NextResponse.json(
        { error: "Missing required fields: pluginName, actionId, approvalId" },
        { status: 400 }
      );
    }

    // Get approval details
    const approval = await db.query.approvals.findFirst({
      where: eq(approvals.id, approvalId),
    });

    if (!approval) {
      return NextResponse.json(
        { error: "Approval not found" },
        { status: 404 }
      );
    }

    // Get task and workspace
    const task = await db.query.tasks.findFirst({
      where: eq(tasks.id, approval.taskId),
    });

    const workspace = await db.query.workspaces.findFirst({
      where: eq(workspaces.id, approval.workspaceId),
    });

    if (!task || !workspace) {
      return NextResponse.json(
        { error: "Task or workspace not found" },
        { status: 404 }
      );
    }

    // Get plugin and its actions
    const registry = getPluginRegistry();
    const plugin = registry.getPlugin(pluginName);

    if (!plugin || !plugin.enabled) {
      return NextResponse.json(
        { error: "Plugin not found or disabled" },
        { status: 404 }
      );
    }

    // Find the action
    const events = getPluginEvents();
    const approvalActions = await events.getApprovalActions();
    const actionDef = approvalActions.find(
      a => a.pluginName === pluginName && a.action.id === actionId
    );

    if (!actionDef) {
      return NextResponse.json(
        { error: "Action not found" },
        { status: 404 }
      );
    }

    // Create context and execute action
    const context = await createPluginContext(plugin);
    const actionContext = {
      approval: {
        id: approval.id,
        branchName: approval.branchName,
        baseBranch: approval.baseBranch,
        commitMessages: approval.commitMessages ? JSON.parse(approval.commitMessages) : [],
        diffSummary: approval.diffSummary ? JSON.parse(approval.diffSummary) : null,
      },
      task: {
        id: task.id,
        title: task.title,
        description: task.description,
        state: task.state,
      },
      workspace: {
        id: workspace.id,
        name: workspace.name,
        repoPath: workspace.repoPath,
      },
      pluginContext: context,
    };

    const result = await actionDef.action.handler(actionContext);

    return NextResponse.json({
      success: result.success,
      message: result.message,
      data: result.data,
    });
  } catch (error) {
    console.error("Failed to execute plugin action:", error);
    return NextResponse.json(
      { error: `Failed to execute action: ${error}` },
      { status: 500 }
    );
  }
}
