import type { Config } from "tailwindcss";

/**
 * Slint-era design tokens, ported. 4-tier background hierarchy (canvas →
 * surface → elevated → overlay) so layered components read visually distinct
 * without ad-hoc neutral-XXX choices. Author color tokens (brian/rain/emma/
 * user) keep chat author dots + accent rings consistent.
 */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      fontFamily: {
        sans: [
          "Inter",
          "ui-sans-serif",
          "system-ui",
          "-apple-system",
          "sans-serif",
        ],
        mono: [
          "JetBrains Mono",
          "ui-monospace",
          "SFMono-Regular",
          "Menlo",
          "monospace",
        ],
      },
      colors: {
        canvas: "#0a0a0a", // app root
        surface: "#141414", // cards, banners
        elevated: "#1d1d1d", // hover states, selected
        overlay: "#262626", // dropdowns, modals
        author: {
          brian: "#f97316",
          rain: "#a855f7",
          emma: "#22c55e",
          user: "#3b82f6",
        },
        accent: {
          DEFAULT: "#3b82f6",
          subtle: "#3b82f622",
        },
      },
      borderColor: {
        DEFAULT: "#262626",
        subtle: "#1d1d1d",
      },
    },
  },
  plugins: [],
} satisfies Config;
