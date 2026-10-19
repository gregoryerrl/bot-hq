import type { Config } from "tailwindcss";

/**
 * Design tokens — the Industrial Terminal palette (`docs/design/industrial_
 * terminal/DESIGN.md`) plus the participant palette. Author colour tokens are
 * named by COLOUR (rc3 D20) and keep chat author dots + accent rings consistent.
 *
 * The 2026-05 migration's legacy 4-tier palette (`canvas`/`elevated`/`overlay`,
 * `accent`, `stack-md`) is gone (round 7, 2026-08-17): the screen batches it
 * was kept for finished, and no class used it. Of the Industrial Terminal
 * tokens below, roughly half have no consumer yet — they stay as the design
 * system's contract for screens not built, deliberately, not as leftovers.
 * The only intentional value collisions with Tailwind's defaults are
 * `outline` (→ #a78b7c) and `borderRadius.DEFAULT` (0.25rem → 0.125rem) —
 * both lean into the design.
 */
export default {
  // Test files are never rendered, and a guard's fixtures spell the classes it
  // bans (`lib/typeScale.test.ts`) — scanned, they would ship as dead CSS.
  content: ["./index.html", "./src/**/*.{ts,tsx}", "!./src/**/*.test.{ts,tsx}"],
  theme: {
    extend: {
      fontFamily: {
        // Kept for Preflight, not for classes: `sans` is the page font (html)
        // and `mono` the font of a bare <code>/<kbd>/<samp>/<pre>. Classes use
        // the semantic families below — `font-mono` duplicated `font-code-sm`,
        // and `lib/typeScale.test.ts` refuses it.
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
        // DESIGN.md's stack and panel tokens (present in the spec since the
        // start, absent here until the 2026-10-19 visual pass).
        "stack-tight": "0.25rem",
        "stack-md": "0.5rem",
        "panel-padding": "1rem",
      },
      colors: {
        surface: "#0b1326",
        author: {
          // **The participant palette** (rc3 D20). Eight hues the ring rotates
          // through by turn slot, and the set a user picks from per participant.
          //
          // Named by COLOUR, never by agent. These were `brian` and `rain` —
          // palette entries carrying two agent names into the design system,
          // which rc3 D10 retired everywhere else. Eight rather than four
          // because a picker wants more choices than the roster cap, and
          // because a palette the same size as the cap leaves a user who
          // dislikes one hue with no alternative that is still distinct.
          //
          // Chosen for separation on the dark surface (#0b1326) — adjacent
          // entries are far apart in hue, so the default rotation looks
          // deliberate at any roster size.
          //
          // **Tuned 2026-10-19 (the visual pass): the hexes, never the names.**
          // They were stock Tailwind 400/500 colours, louder than the role
          // tokens beside them. Now they sit at the theme's pastel lightness
          // (primary #ffb68b, secondary #ddb7ff, tertiary #adc6ff). A
          // participant's override is STORED BY NAME (migration 0052), so a
          // renamed entry would silently fall back to the rotation.
          orange: "#ffa66e",
          violet: "#cfa8ff",
          cyan: "#6fdcec",
          rose: "#ff8fa3",
          lime: "#bce57a",
          amber: "#ffd27a",
          // Darker than the user's #adc6ff (contrast 1.35) and cyan (1.44):
          // at #86c8ff a seventh participant read as you (1.05, EYES).
          sky: "#5fb0ff",
          pink: "#ffa3d4",
          // You — the theme's tertiary blue, not Tailwind's blue-500.
          user: "#adc6ff",
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

        // Primary (the executing role / orange)
        primary: "#ffb68b",
        "on-primary": "#522300",
        "primary-container": "#ff7a00",
        "on-primary-container": "#5c2800",
        "primary-fixed": "#ffdbc8",
        "primary-fixed-dim": "#ffb68b",
        "on-primary-fixed": "#321200",
        "on-primary-fixed-variant": "#753400",
        "inverse-primary": "#994700",

        // Secondary (the reviewing role / purple)
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

        // Secondary text levels (DESIGN.md amendment 3, 2026-10-19): `muted`
        // for hint and help text (a neutral slate — the beige
        // `on-surface-variant` stays for labels), `subtle` for the
        // least-important metadata, `hairline` for quiet dividers. Use these
        // instead of opacity variants of `on-surface-variant`.
        muted: "#a3adc4",
        // >= 4.5:1 up to surface-container-high (5.47 on -container, 4.77
        // on -high; #7a849c was 4.38 / 3.82, EYES fb236827). It reads 4.12
        // on surface-container-highest: keep `subtle` off that surface.
        subtle: "#8b95ad",
        hairline: "#262f45",
      },
      borderColor: {
        // A border with no colour class: the theme's hairline, not neutral grey.
        DEFAULT: "#262f45",
      },
    },
  },
  plugins: [],
} satisfies Config;
