import { NextRequest } from "next/server";
import { ptyManager } from "@/lib/pty-manager";

export const dynamic = "force-dynamic";

export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;

  const session = ptyManager.getSession(id);
  if (!session) {
    return new Response("Session not found", { status: 404 });
  }

  const encoder = new TextEncoder();

  const stream = new ReadableStream({
    start(controller) {
      const onData = (data: string) => {
        try {
          // Send data as SSE event
          const message = `data: ${JSON.stringify({ type: "data", data })}\n\n`;
          controller.enqueue(encoder.encode(message));
        } catch {
          // Stream may be closed
        }
      };

      const onExit = (exitCode: number) => {
        try {
          const message = `data: ${JSON.stringify({ type: "exit", exitCode })}\n\n`;
          controller.enqueue(encoder.encode(message));
          controller.close();
        } catch {
          // Stream may be closed
        }
      };

      session.emitter.on("data", onData);
      session.emitter.on("exit", onExit);

      // Send initial connected message
      const connectMsg = `data: ${JSON.stringify({ type: "connected", sessionId: id })}\n\n`;
      controller.enqueue(encoder.encode(connectMsg));

      // Cleanup on abort
      request.signal.addEventListener("abort", () => {
        session.emitter.off("data", onData);
        session.emitter.off("exit", onExit);
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
