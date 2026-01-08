/**
 * Next.js Instrumentation Hook
 * This file is called when the server starts up
 */

export async function register() {
  if (process.env.NEXT_RUNTIME === "nodejs") {
    const { initializeAgentDocs } = await import("@/lib/agent-docs");
    await initializeAgentDocs();
  }
}
