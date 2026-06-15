import type { Config } from "tailwindcss";

/**
 * Design tokens. 4-tier background hierarchy (canvas →
 * surface → elevated → overlay) so layered components read visually distinct
 * without ad-hoc neutral-XXX choices. Author color tokens (brian/rain/user)
 * keep chat author dots + accent rings consistent.
 *
 * 2026-05 Industrial Terminal migration — additive only. New tokens added
 * alongside the legacy palette; screen batches migrate one surface at a time
 * before the legacy tokens get swept in the cleanup batch. The only
 * intentional value collisions are `surface` (#141414 → #0b1326), `outline`
 * (Tailwind default → #a78b7c), and `borderRadius.DEFAULT` (0.25rem →
 * 0.125rem) — all three lean into the new design.
 */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      fontFamily: {
        // Legacy
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
        // Industrial Terminal semantic families
        "headline-lg": ["Hanken Grotesk", "Inter", "sans-serif"],
        "headline-md": ["Hanken Grotesk", "Inter", "sans-serif"],
        "body-md": ["Inter", "ui-sans-serif", "system-ui", "sans-serif"],
        "code-sm": ["JetBrains Mono", "ui-monospace", "monospace"],
        "label-caps": ["JetBrains Mono", "ui-monospace", "monospace"],
      },
      fontSize: {
        // Industrial Terminal scale. `extend` keeps Tailwind's xs/sm/base/etc.
        "headline-lg": [
          "24px",
          { lineHeight: "32px", fontWeight: "700", letterSpacing: "-0.02em" },
        ],
        "headline-md": [
          "18px",
          { lineHeight: "24px", fontWeight: "600" },
        ],
        "body-md": ["14px", { lineHeight: "20px", fontWeight: "400" }],
        "code-sm": ["12px", { lineHeight: "18px", fontWeight: "400" }],
        "label-caps": [
          "11px",
          { lineHeight: "16px", fontWeight: "700", letterSpacing: "0.05em" },
        ],
      },
      borderRadius: {
        sm: "0.125rem",
        DEFAULT: "0.125rem",
        md: "0.375rem",
        lg: "0.5rem",
        xl: "0.75rem",
        full: "9999px",
      },
      spacing: {
        "grid-margin": "1rem",
        gutter: "0.75rem",
        "stack-md": "0.5rem",
      },
      colors: {
        // Legacy (kept through screen migration)
        canvas: "#0a0a0a",
        surface: "#0b1326", // intentional shift to design value
        elevated: "#1d1d1d",
        overlay: "#262626",
        author: {
          brian: "#f97316",
          rain: "#a855f7",
          user: "#3b82f6",
        },
        accent: {
          DEFAULT: "#3b82f6",
          subtle: "#3b82f622",
        },

        // Industrial Terminal — surface hierarchy
        background: "#0b1326",
        "on-background": "#dae2fd",
        "surface-dim": "#0b1326",
        "surface-bright": "#31394d",
        "surface-container-lowest": "#060e20",
        "surface-container-low": "#131b2e",
        "surface-container": "#171f33",
        "surface-container-high": "#222a3d",
        "surface-container-highest": "#2d3449",
        "on-surface": "#dae2fd",
        "on-surface-variant": "#e0c0af",
        "surface-variant": "#2d3449",
        "surface-tint": "#ffb68b",

        // Outline (shadows Tailwind's default `outline` color — intentional)
        outline: "#a78b7c",
        "outline-variant": "#584235",

        // Primary (Brian / execution / orange)
        primary: "#ffb68b",
        "on-primary": "#522300",
        "primary-container": "#ff7a00",
        "on-primary-container": "#5c2800",
        "primary-fixed": "#ffdbc8",
        "primary-fixed-dim": "#ffb68b",
        "on-primary-fixed": "#321200",
        "on-primary-fixed-variant": "#753400",
        "inverse-primary": "#994700",

        // Secondary (Rain / review / purple)
        secondary: "#ddb7ff",
        "on-secondary": "#490080",
        "secondary-container": "#6f00be",
        "on-secondary-container": "#d6a9ff",
        "secondary-fixed": "#f0dbff",
        "secondary-fixed-dim": "#ddb7ff",
        "on-secondary-fixed": "#2c0051",
        "on-secondary-fixed-variant": "#6900b3",

        // Tertiary (User / input / blue)
        tertiary: "#adc6ff",
        "on-tertiary": "#002e6a",
        "tertiary-container": "#6d9fff",
        "on-tertiary-container": "#003577",
        "tertiary-fixed": "#d8e2ff",
        "tertiary-fixed-dim": "#adc6ff",
        "on-tertiary-fixed": "#001a42",
        "on-tertiary-fixed-variant": "#004395",

        // Error
        error: "#ffb4ab",
        "on-error": "#690005",
        "error-container": "#93000a",
        "on-error-container": "#ffdad6",

        // Success (positive — OK / running / auto-allow / saved / diff-add)
        success: "#7fd99a",
        "on-success": "#00391c",
        "success-container": "#1f5236",
        "on-success-container": "#9bf6b4",

        // Warning (caution — retrying / unsaved / dirty / kept / gated)
        warning: "#f3c150",
        "on-warning": "#3d2e00",
        "warning-container": "#574419",
        "on-warning-container": "#ffdf9e",

        // Inverse
        "inverse-surface": "#dae2fd",
        "inverse-on-surface": "#283044",
      },
      borderColor: {
        DEFAULT: "#262626",
      },
    },
  },
  plugins: [],
} satisfies Config;
