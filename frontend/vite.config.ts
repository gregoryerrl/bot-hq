import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "path";

export default defineConfig({
  plugins: [react()],
  // Tauri serves the bundled frontend from a non-root origin; absolute
  // asset URLs (`/assets/...`) fail with "Could not connect to the server"
  // in the production webview. Relative paths resolve correctly.
  base: "./",
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: ["./src/test-setup.ts"],
    // Vitest replaces CSS imports with empty strings unless told otherwise;
    // `lib/fonts.test.ts` reads the real `index.css` (raw) to check every
    // `@font-face` source against `public/`, so that one file is let through.
    css: { include: [/src\/index\.css/] },
  },
});
