import { NextRequest } from "next/server";
import { chatWithManager, ChatMessage } from "@/lib/agents/manager";

export const dynamic = "force-dynamic";

export async function POST(request: NextRequest) {
  try {
    const { messages } = await request.json() as { messages: ChatMessage[] };

    if (!messages || !Array.isArray(messages)) {
      return new Response(
        JSON.stringify({ error: "messages array is required" }),
        { status: 400, headers: { "Content-Type": "application/json" } }
      );
    }

    const encoder = new TextEncoder();

    const stream = new ReadableStream({
      async start(controller) {
        try {
          await chatWithManager(messages, (chunk) => {
            controller.enqueue(encoder.encode(`data: ${JSON.stringify({ text: chunk })}\n\n`));
          });
          controller.enqueue(encoder.encode(`data: ${JSON.stringify({ done: true })}\n\n`));
          controller.close();
        } catch {
          controller.enqueue(
            encoder.encode(`data: ${JSON.stringify({ error: "Chat failed" })}\n\n`)
          );
          controller.close();
        }
      },
    });

    return new Response(stream, {
      headers: {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-cache",
        Connection: "keep-alive",
      },
    });
  } catch (error) {
    console.error("Manager chat error:", error);
    return new Response(
      JSON.stringify({ error: "Failed to process chat" }),
      { status: 500, headers: { "Content-Type": "application/json" } }
    );
  }
}
