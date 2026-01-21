import { NextRequest, NextResponse } from "next/server";
import { db, gitRemotes } from "@/lib/db";
import { eq } from "drizzle-orm";

function encryptCredentials(token: string): string {
  return Buffer.from(JSON.stringify({ token })).toString("base64");
}

export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const remoteId = parseInt(id);

    const remote = await db.query.gitRemotes.findFirst({
      where: eq(gitRemotes.id, remoteId),
    });

    if (!remote) {
      return NextResponse.json(
        { error: "Remote not found" },
        { status: 404 }
      );
    }

    return NextResponse.json({
      id: remote.id,
      provider: remote.provider,
      name: remote.name,
      url: remote.url,
      owner: remote.owner,
      repo: remote.repo,
      workspaceId: remote.workspaceId,
      isDefault: remote.isDefault,
      hasCredentials: !!remote.credentials,
      createdAt: remote.createdAt?.toISOString(),
    });
  } catch (error) {
    console.error("Failed to fetch git remote:", error);
    return NextResponse.json(
      { error: "Failed to fetch git remote" },
      { status: 500 }
    );
  }
}

export async function PATCH(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const remoteId = parseInt(id);
    const body = await request.json();

    const existing = await db.query.gitRemotes.findFirst({
      where: eq(gitRemotes.id, remoteId),
    });

    if (!existing) {
      return NextResponse.json(
        { error: "Remote not found" },
        { status: 404 }
      );
    }

    const updates: Record<string, any> = { updatedAt: new Date() };

    if (body.name !== undefined) updates.name = body.name;
    if (body.url !== undefined) updates.url = body.url;
    if (body.owner !== undefined) updates.owner = body.owner;
    if (body.repo !== undefined) updates.repo = body.repo;
    if (body.token !== undefined) {
      updates.credentials = body.token ? encryptCredentials(body.token) : null;
    }

    if (body.isDefault === true) {
      // Unset other defaults for this provider
      await db
        .update(gitRemotes)
        .set({ isDefault: false })
        .where(eq(gitRemotes.provider, existing.provider));
      updates.isDefault = true;
    }

    await db.update(gitRemotes).set(updates).where(eq(gitRemotes.id, remoteId));

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to update git remote:", error);
    return NextResponse.json(
      { error: "Failed to update git remote" },
      { status: 500 }
    );
  }
}

export async function DELETE(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const remoteId = parseInt(id);

    await db.delete(gitRemotes).where(eq(gitRemotes.id, remoteId));

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to delete git remote:", error);
    return NextResponse.json(
      { error: "Failed to delete git remote" },
      { status: 500 }
    );
  }
}
