// src/app/api/plugins/route.ts

import { NextResponse } from "next/server";
import { getPluginRegistry, initializePlugins } from "@/lib/plugins";

export async function GET() {
  try {
    const registry = getPluginRegistry();

    if (!registry.isInitialized()) {
      await initializePlugins();
    }

    const plugins = registry.getAllPlugins().map(p => ({
      name: p.name,
      version: p.version,
      description: p.manifest.description,
      enabled: p.enabled,
      hasUI: !!p.manifest.ui,
      hasMcp: !!p.manifest.mcp,
    }));

    return NextResponse.json({ plugins });
  } catch (error) {
    console.error("Failed to list plugins:", error);
    return NextResponse.json(
      { error: "Failed to list plugins" },
      { status: 500 }
    );
  }
}
