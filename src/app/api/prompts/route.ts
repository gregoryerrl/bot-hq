import { NextResponse } from "next/server";
import { getAllPrompts, seedDefaultPrompts } from "@/lib/prompts";

export async function GET() {
  try {
    // Seed defaults if DB is empty
    await seedDefaultPrompts();
    const allPrompts = await getAllPrompts();
    return NextResponse.json(allPrompts);
  } catch (error) {
    console.error("Failed to fetch prompts:", error);
    return NextResponse.json(
      { error: "Failed to fetch prompts" },
      { status: 500 }
    );
  }
}
