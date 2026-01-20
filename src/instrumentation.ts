/**
 * Next.js Instrumentation Hook
 * This file is called when the server starts up
 */

export async function register() {
  if (process.env.NEXT_RUNTIME === "nodejs") {
    const { initializeAgentDocs } = await import("@/lib/agent-docs");
    await initializeAgentDocs();

    // Initialize plugins
    try {
      const { initializePlugins } = await import("@/lib/plugins");
      await initializePlugins();
      console.log("Plugins initialized");
    } catch (error) {
      console.error("Failed to initialize plugins:", error);
    }

    // Start persistent manager
    try {
      const { startManager } = await import("@/lib/manager/persistent-manager");
      await startManager();
      console.log("Manager session started");
    } catch (error) {
      console.error("Failed to start manager:", error);
    }
  }
}
