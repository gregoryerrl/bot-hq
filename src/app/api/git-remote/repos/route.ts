import { NextResponse } from "next/server";
import { db, gitRemotes } from "@/lib/db";
import { eq } from "drizzle-orm";

function decryptCredentials(encrypted: string): { token: string } | null {
  try {
    return JSON.parse(Buffer.from(encrypted, "base64").toString("utf-8"));
  } catch {
    return null;
  }
}

interface GitHubRepo {
  name: string;
  full_name: string;
  description: string | null;
  private: boolean;
  html_url: string;
  owner: { login: string };
}

export async function GET() {
  try {
    // Find a GitHub remote with credentials
    const remote = await db.query.gitRemotes.findFirst({
      where: eq(gitRemotes.provider, "github"),
    });

    if (!remote?.credentials) {
      return NextResponse.json(
        { error: "No GitHub credentials configured", repos: [] },
        { status: 200 }
      );
    }

    const creds = decryptCredentials(remote.credentials);
    if (!creds?.token) {
      return NextResponse.json(
        { error: "Invalid credentials", repos: [] },
        { status: 200 }
      );
    }

    // Fetch repos from GitHub
    const response = await fetch(
      "https://api.github.com/user/repos?per_page=100&sort=updated",
      {
        headers: {
          Authorization: `token ${creds.token}`,
          Accept: "application/vnd.github.v3+json",
          "User-Agent": "bot-hq",
        },
      }
    );

    if (!response.ok) {
      return NextResponse.json(
        { error: "Failed to fetch repositories from GitHub", repos: [] },
        { status: 200 }
      );
    }

    const repos: GitHubRepo[] = await response.json();

    return NextResponse.json({
      repos: repos.map((repo) => ({
        owner: repo.owner.login,
        name: repo.name,
        fullName: repo.full_name,
        description: repo.description,
        private: repo.private,
        url: repo.html_url,
      })),
    });
  } catch (error) {
    console.error("Failed to fetch repos:", error);
    return NextResponse.json(
      { error: "Failed to fetch repositories", repos: [] },
      { status: 500 }
    );
  }
}
