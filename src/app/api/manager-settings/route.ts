import { NextResponse } from "next/server";
import { db, settings } from "@/lib/db";
import { eq } from "drizzle-orm";
import { getManagerPrompt, saveManagerPrompt } from "@/lib/bot-hq";

interface ManagerSettings {
  managerPrompt: string;
  maxIterations: number;
  stuckThreshold: number;
}

export async function GET() {
  try {
    // Get settings from database
    const maxIterationsRow = await db.query.settings.findFirst({
      where: eq(settings.key, "max_iterations"),
    });
    const stuckThresholdRow = await db.query.settings.findFirst({
      where: eq(settings.key, "stuck_threshold"),
    });

    // Get manager prompt from file
    const managerPrompt = await getManagerPrompt();

    const response: ManagerSettings = {
      managerPrompt,
      maxIterations: maxIterationsRow ? parseInt(maxIterationsRow.value) : 10,
      stuckThreshold: stuckThresholdRow ? parseInt(stuckThresholdRow.value) : 3,
    };

    return NextResponse.json(response);
  } catch (error) {
    console.error("Failed to get manager settings:", error);
    return NextResponse.json(
      { error: "Failed to get settings" },
      { status: 500 }
    );
  }
}

export async function PUT(request: Request) {
  try {
    const body: Partial<ManagerSettings> = await request.json();

    // Update manager prompt file
    if (body.managerPrompt !== undefined) {
      await saveManagerPrompt(body.managerPrompt);
    }

    // Update database settings
    if (body.maxIterations !== undefined) {
      await db
        .insert(settings)
        .values({
          key: "max_iterations",
          value: String(body.maxIterations),
        })
        .onConflictDoUpdate({
          target: settings.key,
          set: { value: String(body.maxIterations), updatedAt: new Date() },
        });
    }

    if (body.stuckThreshold !== undefined) {
      await db
        .insert(settings)
        .values({
          key: "stuck_threshold",
          value: String(body.stuckThreshold),
        })
        .onConflictDoUpdate({
          target: settings.key,
          set: { value: String(body.stuckThreshold), updatedAt: new Date() },
        });
    }

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to update manager settings:", error);
    return NextResponse.json(
      { error: "Failed to update settings" },
      { status: 500 }
    );
  }
}
