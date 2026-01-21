import { NextResponse } from "next/server";
import { db, plugins } from "@/lib/db";
import { initializePlugins, getPluginRegistry } from "@/lib/plugins";
import { eq } from "drizzle-orm";

export async function GET() {
  try {
    await initializePlugins();

    const plugin = await db.query.plugins.findFirst({
      where: eq(plugins.name, "github"),
    });

    if (!plugin) {
      return NextResponse.json({ hasToken: false, connected: false });
    }

    const credentials = plugin.credentials ? JSON.parse(plugin.credentials) : {};
    const hasToken = !!credentials.GITHUB_TOKEN;

    if (!hasToken) {
      return NextResponse.json({ hasToken: false, connected: false });
    }

    // Test the connection
    try {
      const res = await fetch("https://api.github.com/user", {
        headers: {
          Authorization: `Bearer ${credentials.GITHUB_TOKEN}`,
          Accept: "application/vnd.github+json",
          "X-GitHub-Api-Version": "2022-11-28",
        },
      });

      const connected = res.ok;
      return NextResponse.json({ hasToken: true, connected });
    } catch {
      return NextResponse.json({ hasToken: true, connected: false });
    }
  } catch (error) {
    console.error("Failed to get credentials:", error);
    return NextResponse.json(
      { error: "Failed to get credentials" },
      { status: 500 }
    );
  }
}

export async function POST(request: Request) {
  try {
    const { token } = await request.json();

    if (!token || typeof token !== "string") {
      return NextResponse.json(
        { error: "Token is required" },
        { status: 400 }
      );
    }

    // Validate the token by making a test request
    const testRes = await fetch("https://api.github.com/user", {
      headers: {
        Authorization: `Bearer ${token}`,
        Accept: "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
      },
    });

    const connected = testRes.ok;

    await initializePlugins();
    const registry = getPluginRegistry();

    // Save the token
    await registry.updateCredentials("github", { GITHUB_TOKEN: token });

    return NextResponse.json({
      success: true,
      connected,
      message: connected
        ? "Token saved and connection verified"
        : "Token saved but connection failed - please check the token",
    });
  } catch (error) {
    console.error("Failed to save credentials:", error);
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to save credentials" },
      { status: 500 }
    );
  }
}
