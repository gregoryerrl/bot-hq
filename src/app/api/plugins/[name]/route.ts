// src/app/api/plugins/[name]/route.ts

import { NextResponse } from "next/server";
import { getPluginRegistry } from "@/lib/plugins";

export async function GET(
  request: Request,
  { params }: { params: Promise<{ name: string }> }
) {
  try {
    const { name } = await params;
    const registry = getPluginRegistry();
    const plugin = registry.getPlugin(name);

    if (!plugin) {
      return NextResponse.json(
        { error: "Plugin not found" },
        { status: 404 }
      );
    }

    const settings = await registry.getSettings(name);

    return NextResponse.json({
      name: plugin.name,
      version: plugin.version,
      description: plugin.manifest.description,
      enabled: plugin.enabled,
      manifest: plugin.manifest,
      settings,
    });
  } catch (error) {
    console.error("Failed to get plugin:", error);
    return NextResponse.json(
      { error: "Failed to get plugin" },
      { status: 500 }
    );
  }
}

export async function PATCH(
  request: Request,
  { params }: { params: Promise<{ name: string }> }
) {
  try {
    const { name } = await params;
    const body = await request.json();
    const registry = getPluginRegistry();

    if (typeof body.enabled === "boolean") {
      await registry.setEnabled(name, body.enabled);
    }

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to update plugin:", error);
    return NextResponse.json(
      { error: "Failed to update plugin" },
      { status: 500 }
    );
  }
}
