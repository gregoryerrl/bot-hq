// src/app/api/plugins/[name]/settings/route.ts

import { NextResponse } from "next/server";
import { getPluginRegistry } from "@/lib/plugins";

export async function GET(
  request: Request,
  { params }: { params: Promise<{ name: string }> }
) {
  try {
    const { name } = await params;
    const registry = getPluginRegistry();
    const settings = await registry.getSettings(name);

    return NextResponse.json({ settings });
  } catch (error) {
    console.error("Failed to get plugin settings:", error);
    return NextResponse.json(
      { error: "Failed to get settings" },
      { status: 500 }
    );
  }
}

export async function PUT(
  request: Request,
  { params }: { params: Promise<{ name: string }> }
) {
  try {
    const { name } = await params;
    const body = await request.json();
    const registry = getPluginRegistry();

    await registry.updateSettings(name, body.settings || {});

    if (body.credentials) {
      await registry.updateCredentials(name, body.credentials);
    }

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to update plugin settings:", error);
    return NextResponse.json(
      { error: "Failed to update settings" },
      { status: 500 }
    );
  }
}
