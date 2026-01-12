// src/app/api/plugins/actions/route.ts

import { NextRequest, NextResponse } from "next/server";
import { getPluginEvents } from "@/lib/plugins";

export async function GET(request: NextRequest) {
  try {
    const searchParams = request.nextUrl.searchParams;
    const type = searchParams.get("type"); // "approval" | "task" | "workspace"

    const events = getPluginEvents();
    let actions;

    switch (type) {
      case "approval":
        actions = await events.getApprovalActions();
        break;
      case "task":
        actions = await events.getTaskActions();
        break;
      default:
        return NextResponse.json(
          { error: "Invalid action type. Use 'approval', 'task', or 'workspace'" },
          { status: 400 }
        );
    }

    return NextResponse.json({
      actions: actions.map(a => ({
        pluginName: a.pluginName,
        id: a.action.id,
        label: a.action.label,
        description: typeof a.action.description === "string"
          ? a.action.description
          : undefined,
        icon: a.action.icon,
        defaultChecked: a.action.defaultChecked ?? false,
      })),
    });
  } catch (error) {
    console.error("Failed to get plugin actions:", error);
    return NextResponse.json(
      { error: "Failed to get plugin actions" },
      { status: 500 }
    );
  }
}
