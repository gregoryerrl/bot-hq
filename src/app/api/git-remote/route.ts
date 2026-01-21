import { NextRequest, NextResponse } from "next/server";
import { db, gitRemotes, workspaces } from "@/lib/db";
import { eq } from "drizzle-orm";

// Encrypt token (simple base64 for now - should use proper encryption in production)
function encryptCredentials(token: string): string {
  return Buffer.from(JSON.stringify({ token })).toString("base64");
}

export async function GET() {
  try {
    const remotes = await db.select().from(gitRemotes);

    // Enrich with workspace names and check credentials
    const enriched = await Promise.all(
      remotes.map(async (remote) => {
        let workspaceName: string | undefined;
        if (remote.workspaceId) {
          const ws = await db.query.workspaces.findFirst({
            where: eq(workspaces.id, remote.workspaceId),
          });
          workspaceName = ws?.name;
        }

        return {
          id: remote.id,
          workspaceId: remote.workspaceId,
          provider: remote.provider,
          name: remote.name,
          url: remote.url,
          owner: remote.owner,
          repo: remote.repo,
          isDefault: remote.isDefault,
          hasCredentials: !!remote.credentials,
          workspaceName,
          createdAt: remote.createdAt?.toISOString(),
        };
      })
    );

    return NextResponse.json(enriched);
  } catch (error) {
    console.error("Failed to fetch git remotes:", error);
    return NextResponse.json(
      { error: "Failed to fetch git remotes" },
      { status: 500 }
    );
  }
}

export async function POST(request: NextRequest) {
  try {
    const body = await request.json();
    const { provider, name, url, workspaceId, owner, repo, token, isDefault } = body;

    if (!provider || !name || !url) {
      return NextResponse.json(
        { error: "Missing required fields: provider, name, url" },
        { status: 400 }
      );
    }

    // If setting as default, unset other defaults for this provider
    if (isDefault) {
      await db
        .update(gitRemotes)
        .set({ isDefault: false })
        .where(eq(gitRemotes.provider, provider));
    }

    const credentials = token ? encryptCredentials(token) : null;

    const [newRemote] = await db
      .insert(gitRemotes)
      .values({
        provider,
        name,
        url,
        workspaceId: workspaceId || null,
        owner: owner || null,
        repo: repo || null,
        credentials,
        isDefault: isDefault || false,
        createdAt: new Date(),
        updatedAt: new Date(),
      })
      .returning();

    return NextResponse.json({
      id: newRemote.id,
      provider: newRemote.provider,
      name: newRemote.name,
      hasCredentials: !!newRemote.credentials,
    });
  } catch (error) {
    console.error("Failed to create git remote:", error);
    return NextResponse.json(
      { error: "Failed to create git remote" },
      { status: 500 }
    );
  }
}
