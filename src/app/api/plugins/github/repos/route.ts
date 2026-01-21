import { NextResponse } from "next/server";
import { db, plugins } from "@/lib/db";
import { initializePlugins } from "@/lib/plugins";
import { eq } from "drizzle-orm";

interface GitHubRepo {
  id: number;
  name: string;
  full_name: string;
  description: string | null;
  private: boolean;
  html_url: string;
  owner: {
    login: string;
  };
}

export async function GET() {
  try {
    await initializePlugins();

    const plugin = await db.query.plugins.findFirst({
      where: eq(plugins.name, "github"),
    });

    if (!plugin) {
      return NextResponse.json(
        { error: "GitHub plugin not installed" },
        { status: 400 }
      );
    }

    const credentials = plugin.credentials ? JSON.parse(plugin.credentials) : {};
    const token = credentials.GITHUB_TOKEN;

    if (!token) {
      return NextResponse.json(
        { error: "GitHub token not configured" },
        { status: 400 }
      );
    }

    // Fetch user's repositories from GitHub
    const repos: GitHubRepo[] = [];
    let page = 1;
    const perPage = 100;

    while (true) {
      const res = await fetch(
        `https://api.github.com/user/repos?per_page=${perPage}&page=${page}&sort=updated&direction=desc`,
        {
          headers: {
            Authorization: `Bearer ${token}`,
            Accept: "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
          },
        }
      );

      if (!res.ok) {
        if (res.status === 401) {
          return NextResponse.json(
            { error: "GitHub token is invalid or expired" },
            { status: 401 }
          );
        }
        throw new Error(`GitHub API error: ${res.statusText}`);
      }

      const pageRepos: GitHubRepo[] = await res.json();
      repos.push(...pageRepos);

      // Check if we've fetched all repos
      if (pageRepos.length < perPage) {
        break;
      }

      page++;

      // Safety limit - don't fetch more than 500 repos
      if (repos.length >= 500) {
        break;
      }
    }

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
      { error: error instanceof Error ? error.message : "Failed to fetch repositories" },
      { status: 500 }
    );
  }
}
