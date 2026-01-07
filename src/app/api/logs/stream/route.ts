import { NextRequest } from "next/server";
import { db, logs, agentSessions } from "@/lib/db";
import { desc, gt, ne, eq, and } from "drizzle-orm";

export const dynamic = "force-dynamic";

export async function GET(request: NextRequest) {
  const { searchParams } = new URL(request.url);
  const source = searchParams.get("source") || "all";
  const sessionId = searchParams.get("sessionId");

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
          let query;

          if (source === "server") {
            // Server logs: everything except agent type
            query = db
              .select()
              .from(logs)
              .where(
                and(
                  gt(logs.id, lastId),
                  ne(logs.type, "agent")
                )
              )
              .orderBy(desc(logs.createdAt))
              .limit(20);
          } else if (source === "agent" && sessionId) {
            // Agent logs: get taskId from session, filter by that
            const session = await db
              .select()
              .from(agentSessions)
              .where(eq(agentSessions.id, parseInt(sessionId)))
              .limit(1);

            if (session[0] && session[0].taskId !== null) {
              const taskId = session[0].taskId;
              query = db
                .select()
                .from(logs)
                .where(
                  and(
                    gt(logs.id, lastId),
                    eq(logs.type, "agent"),
                    eq(logs.taskId, taskId)
                  )
                )
                .orderBy(desc(logs.createdAt))
                .limit(20);
            } else {
              return; // Session not found or no taskId
            }
          } else {
            // All logs (default, for backwards compatibility)
            query = db
              .select()
              .from(logs)
              .where(gt(logs.id, lastId))
              .orderBy(desc(logs.createdAt))
              .limit(20);
          }

          const newLogs = await query;

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
