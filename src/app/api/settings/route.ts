import { NextResponse } from "next/server";
import { getSetting, setSetting, getScopePath, setScopePath } from "@/lib/settings";
import fs from "fs/promises";

export async function GET(request: Request) {
  const { searchParams } = new URL(request.url);
  const key = searchParams.get("key");

  if (!key) {
    return NextResponse.json(
      { error: "Missing 'key' query parameter" },
      { status: 400 }
    );
  }

  try {
    // Special handling for scope_path to include fallback logic
    if (key === "scope_path") {
      const scopePath = await getScopePath();
      return NextResponse.json({ key, value: scopePath });
    }

    const value = await getSetting(key);

    if (value === null) {
      return NextResponse.json(
        { error: `Setting '${key}' not found` },
        { status: 404 }
      );
    }

    return NextResponse.json({ key, value });
  } catch (error) {
    console.error("Failed to get setting:", error);
    return NextResponse.json(
      { error: "Failed to retrieve setting" },
      { status: 500 }
    );
  }
}

export async function PUT(request: Request) {
  try {
    const { key, value } = await request.json();

    if (!key || typeof value !== "string") {
      return NextResponse.json(
        { error: "Missing or invalid 'key' or 'value'" },
        { status: 400 }
      );
    }

    // Special handling for scope_path to validate directory exists
    if (key === "scope_path") {
      try {
        const stats = await fs.stat(value);
        if (!stats.isDirectory()) {
          return NextResponse.json(
            { error: "Path is not a directory" },
            { status: 400 }
          );
        }
      } catch (error) {
        return NextResponse.json(
          { error: "Directory does not exist" },
          { status: 400 }
        );
      }

      await setScopePath(value);
    } else {
      await setSetting(key, value);
    }

    return NextResponse.json({ success: true, key, value });
  } catch (error) {
    console.error("Failed to set setting:", error);
    return NextResponse.json(
      { error: "Failed to save setting" },
      { status: 500 }
    );
  }
}
