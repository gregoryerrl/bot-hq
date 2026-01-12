import { NextRequest, NextResponse } from "next/server";
import { db, workspaces, NewWorkspace } from "@/lib/db";

export async function GET() {
  try {
    const allWorkspaces = await db.select().from(workspaces);
    return NextResponse.json(allWorkspaces);
  } catch (error) {
    console.error("Failed to fetch workspaces:", error);
    return NextResponse.json(
      { error: "Failed to fetch workspaces" },
      { status: 500 }
    );
  }
}

export async function POST(request: NextRequest) {
  try {
    const body = await request.json();

    const newWorkspace: NewWorkspace = {
      name: body.name,
      repoPath: body.repoPath,
      linkedDirs: body.linkedDirs ? JSON.stringify(body.linkedDirs) : null,
      buildCommand: body.buildCommand || null,
    };

    const result = await db.insert(workspaces).values(newWorkspace).returning();
    return NextResponse.json(result[0], { status: 201 });
  } catch (error) {
    console.error("Failed to create workspace:", error);
    return NextResponse.json(
      { error: "Failed to create workspace" },
      { status: 500 }
    );
  }
}
