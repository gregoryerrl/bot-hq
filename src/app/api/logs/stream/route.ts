import { NextRequest } from "next/server";
import { db, logs } from "@/lib/db";
import { desc, gt } from "drizzle-orm";

export const dynamic = "force-dynamic";

export async function GET(request: NextRequest) {
  const encoder = new TextEncoder();
  let lastId = 0;

  const stream = new ReadableStream({
    async start(controller) {
      // Send initial connection message
      controller.enqueue(
        encoder.encode(`data: ${JSON.stringify({ type: "connected" })}\n\n`)
      );

      // Poll for new logs every second
      const interval = setInterval(async () => {
        try {
          const newLogs = await db
            .select()
            .from(logs)
            .where(gt(logs.id, lastId))
            .orderBy(desc(logs.createdAt))
            .limit(20);

          if (newLogs.length > 0) {
            lastId = Math.max(...newLogs.map((l) => l.id));
            for (const log of newLogs.reverse()) {
              controller.enqueue(
                encoder.encode(`data: ${JSON.stringify(log)}\n\n`)
              );
            }
          }
        } catch (error) {
          console.error("SSE error:", error);
        }
      }, 1000);

      // Clean up on close
      request.signal.addEventListener("abort", () => {
        clearInterval(interval);
        controller.close();
      });
    },
  });

  return new Response(stream, {
    headers: {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache",
      Connection: "keep-alive",
    },
  });
}
