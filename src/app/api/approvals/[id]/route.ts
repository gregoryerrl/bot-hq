import { NextRequest, NextResponse } from "next/server";
import { db, approvals, tasks, logs } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const result = await db
      .select()
      .from(approvals)
      .where(eq(approvals.id, parseInt(id)))
      .limit(1);

    if (result.length === 0) {
      return NextResponse.json({ error: "Approval not found" }, { status: 404 });
    }

    return NextResponse.json(result[0]);
  } catch (error) {
    console.error("Failed to fetch approval:", error);
    return NextResponse.json({ error: "Failed to fetch approval" }, { status: 500 });
  }
}

export async function POST(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const { action } = await request.json();

    if (!["approve", "reject"].includes(action)) {
      return NextResponse.json(
        { error: "Invalid action. Use 'approve' or 'reject'" },
        { status: 400 }
      );
    }

    const approval = await db.query.approvals.findFirst({
      where: eq(approvals.id, parseInt(id)),
    });

    if (!approval) {
      return NextResponse.json(
        { error: "Approval not found" },
        { status: 404 }
      );
    }

    if (approval.status !== "pending") {
      return NextResponse.json(
        { error: "Approval already resolved" },
        { status: 400 }
      );
    }

    // Update approval status
    const newStatus = action === "approve" ? "approved" : "rejected";
    await db
      .update(approvals)
      .set({
        status: newStatus,
        resolvedAt: new Date(),
      })
      .where(eq(approvals.id, parseInt(id)));

    // Update task state if taskId exists
    if (approval.taskId) {
      if (action === "approve") {
        await db
          .update(tasks)
          .set({ state: "in_progress", updatedAt: new Date() })
          .where(eq(tasks.id, approval.taskId));
      } else {
        await db
          .update(tasks)
          .set({ state: "queued", updatedAt: new Date() })
          .where(eq(tasks.id, approval.taskId));
      }

      // Log the action
      const task = await db.query.tasks.findFirst({
        where: eq(tasks.id, approval.taskId),
      });

      if (task) {
        await db.insert(logs).values({
          workspaceId: task.workspaceId,
          taskId: task.id,
          type: "approval",
          message: `${action === "approve" ? "Approved" : "Rejected"}: ${approval.command}`,
        });
      }
    } else {
      // Log approval action without task context
      await db.insert(logs).values({
        type: "approval",
        message: `${action === "approve" ? "Approved" : "Rejected"}: ${approval.command}`,
      });
    }

    return NextResponse.json({ status: newStatus });
  } catch (error) {
    console.error("Failed to process approval:", error);
    return NextResponse.json(
      { error: "Failed to process approval" },
      { status: 500 }
    );
  }
}
