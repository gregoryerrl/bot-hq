import { NextRequest, NextResponse } from "next/server";
import { db, taskNotes } from "@/lib/db";
import { eq, asc } from "drizzle-orm";

export async function GET(
  _request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const taskId = Number(id);

  const notes = await db
    .select()
    .from(taskNotes)
    .where(eq(taskNotes.taskId, taskId))
    .orderBy(asc(taskNotes.createdAt));

  return NextResponse.json(notes);
}

export async function POST(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const taskId = Number(id);
  const body = await request.json();

  if (!body.content || typeof body.content !== "string" || body.content.trim() === "") {
    return NextResponse.json({ error: "content is required" }, { status: 400 });
  }

  const [note] = await db
    .insert(taskNotes)
    .values({
      taskId,
      content: body.content.trim(),
    })
    .returning();

  return NextResponse.json(note, { status: 201 });
}
