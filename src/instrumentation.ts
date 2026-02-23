/**
 * Next.js Instrumentation Hook
 * This file is called when the server starts up
 */

export async function register() {
  if (process.env.NEXT_RUNTIME === "nodejs") {
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
