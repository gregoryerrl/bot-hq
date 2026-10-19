---
name: Industrial Terminal
colors:
  surface: '#0b1326'
  surface-dim: '#0b1326'
  surface-bright: '#31394d'
  surface-container-lowest: '#060e20'
  surface-container-low: '#131b2e'
  surface-container: '#171f33'
  surface-container-high: '#222a3d'
  surface-container-highest: '#2d3449'
  on-surface: '#dae2fd'
  on-surface-variant: '#e0c0af'
  inverse-surface: '#dae2fd'
  inverse-on-surface: '#283044'
  outline: '#a78b7c'
  outline-variant: '#584235'
  surface-tint: '#ffb68b'
  primary: '#ffb68b'
  on-primary: '#522300'
  primary-container: '#ff7a00'
  on-primary-container: '#5c2800'
  inverse-primary: '#994700'
  secondary: '#ddb7ff'
  on-secondary: '#490080'
  secondary-container: '#6f00be'
  on-secondary-container: '#d6a9ff'
  tertiary: '#adc6ff'
  on-tertiary: '#002e6a'
  tertiary-container: '#6d9fff'
  on-tertiary-container: '#003577'
  error: '#ffb4ab'
  on-error: '#690005'
  error-container: '#93000a'
  on-error-container: '#ffdad6'
  primary-fixed: '#ffdbc8'
  primary-fixed-dim: '#ffb68b'
  on-primary-fixed: '#321200'
  on-primary-fixed-variant: '#753400'
  secondary-fixed: '#f0dbff'
  secondary-fixed-dim: '#ddb7ff'
  on-secondary-fixed: '#2c0051'
  on-secondary-fixed-variant: '#6900b3'
  tertiary-fixed: '#d8e2ff'
  tertiary-fixed-dim: '#adc6ff'
  on-tertiary-fixed: '#001a42'
  on-tertiary-fixed-variant: '#004395'
  background: '#0b1326'
  on-background: '#dae2fd'
  surface-variant: '#2d3449'
typography:
  headline-lg:
    fontFamily: Hanken Grotesk
    fontSize: 24px
    fontWeight: '700'
    lineHeight: 32px
    letterSpacing: -0.02em
  headline-md:
    fontFamily: Hanken Grotesk
    fontSize: 18px
    fontWeight: '600'
    lineHeight: 24px
  body-md:
    fontFamily: Inter
    fontSize: 14px
    fontWeight: '400'
    lineHeight: 20px
  code-sm:
    fontFamily: JetBrains Mono
    fontSize: 12px
    fontWeight: '400'
    lineHeight: 18px
  label-caps:
    fontFamily: JetBrains Mono
    fontSize: 11px
    fontWeight: '700'
    lineHeight: 16px
    letterSpacing: 0.05em
rounded:
  sm: 0.125rem
  DEFAULT: 0.125rem
  md: 0.375rem
  lg: 0.5rem
  xl: 0.75rem
  full: 9999px
spacing:
  grid-margin: 1rem
  gutter: 0.75rem
  stack-tight: 0.25rem
  stack-md: 0.5rem
  panel-padding: 1rem
---

## Brand & Style

This design system is engineered for high-density bot orchestration and developer operations. The aesthetic is **Industrial Minimalism**—prioritizing information density, mechanical precision, and immediate visual hierarchy. The UI should evoke the feeling of a sophisticated command center: cold, efficient, and uncompromisingly functional.

The target audience consists of developers and system architects who require high-speed monitoring and granular control. By utilizing a dark, low-fatigue palette punctuated by high-chroma status indicators, the system ensures that "phases" and "actors" (the executing role, the reviewing role, the user) are instantly identifiable in a sea of data.

Key visual principles:
- **High Density:** Minimal whitespace between functional modules to maximize data throughput.
- **Precision:** Mathematical alignment and consistent stroke weights.
- **Tactile feedback:** Subtle state changes that mimic physical hardware toggles.

## Colors

The palette is anchored by a "Nearly Black" slate background to reduce eye strain during long-form monitoring. Role-based color coding is the primary driver of the interface's information architecture.

- **Primary (Orange):** the executing role — hands-on tasks and active bot processes. High energy, high visibility. (Since rc3 D20 a participant's colour is one of an 8-hue palette hashed on its `ROLE · Model` label, with a per-participant override; orange/purple are the palette's first two hues, no longer bound to a slot or a name.)
- **Secondary (Purple):** the reviewing role — review, vision, and observation states. Sophisticated and distinct from execution.
- **Tertiary (User/Blue):** Reserved for human input and user-originated actions.
- **System (Muted Grey):** Used for phase changes, background logs, and passive infrastructure updates.
- **Surface:** Surfaces are layered using slightly lighter slate tones (`#0F172A` to `#1E293B`) to create structural separation without relying on heavy shadows.

## Typography

The typography system balances modern UI clarity with the technical rigor of developer tools.

- **UI Headings:** Hanken Grotesk provides a sharp, contemporary feel for top-level navigation and dashboard metrics.
- **Body Content:** Inter is utilized for its exceptional legibility in dense lists and chat threads.
- **Technical Data:** JetBrains Mono is the workhorse for agent logs, code snippets, and system metadata. All labels and status chips use monospaced caps to reinforce the industrial aesthetic.

Use `code-sm` for all chronological chat logs to maintain a "terminal-first" feel.

## Layout & Spacing

The layout follows a **Fixed Dashboard Grid** model for the main workspace, ensuring predictable positioning of monitoring tiles.

1.  **Topbar:** Fixed height (48px), containing global system status and search.
2.  **Dashboard Grid:** A multi-column responsive grid using 0.75rem gutters for "Session Tiles."
3.  **Session View:** A central wide column for chronological chat flow.
4.  **Right-Pane Overlay:** a slide-out panel (320px fixed width) for secondary metadata — the Session Settings panel is the shipped instance. (The original spec called this the "Emma Overlay"; that agent was removed from the core.)

Padding is intentionally tight to facilitate high information density. Internal module spacing should default to 0.5rem (8px).

## Elevation & Depth

Depth is achieved through **Tonal Layering** and **Low-Contrast Outlines** rather than traditional shadows. This maintains the "Industrial" flat-panel aesthetic.

- **Level 0 (Canvas):** Darkest slate (`#020617`).
- **Level 1 (Panels/Tiles):** Mid-slate (`#0F172A`) with a 1px solid border (`#1E293B`).
- **Level 2 (Active/Hover):** Border color shifts to the accent color associated with the role (Orange/Purple/Blue).
- **Right-Pane Overlay:** Uses a subtle backdrop blur (12px) and a slightly more prominent 1px border to indicate its "floating" status over the main dashboard.

## Shapes

The shape language is "Soft-Industrial." Components — chips, inputs, buttons, pills — share the house **2px (0.125rem, `rounded`)** corner radius; dialogs, session tiles and archive rows use `rounded-lg` (8px). Sharp enough to read as an instrument panel, not a consumer app. (The spec first said 4px; the shipped token is 2px and every component was built to it, so round 11 amended the number here rather than re-rounding the app — `Button` md/lg had drifted to 6px and were brought back to `rounded`.)

- **Chips:** Small 2px radius containers for phases.
- **Banners:** Full-width status indicators with 0px radius where they touch container edges.
- **Inputs:** Squared-off with 2px radius for a more technical, "input-field" feel — the same radius as the buttons beside them.

## Components

### Status Chips (Phases)
Small, monospaced labels used to indicate current bot phase (e.g., `PLANNING`, `EXECUTING`). Background color should be a 15% opacity tint of the role color with a 100% opacity text label.

### Session Tiles
The primary dashboard element. Tiles must contain a header with the session ID, a mini sparkline or status indicator, and a "Banner" at the bottom showing the latest system/phase change.

### Banners
Status banners appear at the top of the Session View. Use the **System Muted** grey for passive logs and the **Primary/Secondary** colors for urgent state changes.

### Chat Chronology
A single vertical stream. User messages are right-aligned with Blue accents; Agent messages are left-aligned with their participant's accent hue. Use monospaced font for all message content to emphasize the "data" nature of the bot interaction.

### Inputs
Command-line style inputs. No background, just a bottom border that illuminates with the active role color when focused. Use a block cursor character `█` to reinforce the terminal aesthetic.
## Amendments — 2026-10-19 (the visual pass; the user's picks on tray 41aa6ff8)

These four amendments take precedence over the sections above where they differ.

1. **Message prose is Inter (`body-md`); mono is for activity and data.** Agent and user message text renders in Inter for long-form legibility. JetBrains Mono (`code-sm`, `label-caps`) stays for tool activity, code, ids, model names, timestamps and every other kind of metadata. The "Chat Chronology" rule's *mono for all message content* and the Typography section's *`code-sm` for all chronological chat logs* are superseded.
2. **Fields are boxed.** Input, select and textarea share one style: a 1px outline (`outline-variant`) that lights in the role colour on focus, with a 2px radius. This supersedes the "Inputs" rule's *no background, just a bottom border*, which cannot carry a select or a multi-line field.
3. **Hint and help text is a neutral muted slate (`muted`).** `on-surface-variant` (the warm beige #e0c0af) is kept for labels and captions only, so long explanatory text stops reading as warm and loud. Its type follows its length: a **one-line hint** (one sentence beside or under a control, a banner's last line, a status line) is mono `code-sm`; **multi-line help** (a page's or section's description, a paragraph of two or more sentences) is Inter `body-md`, for amendment 1's reason — Inter reads better long-form.
4. **Document tabs wrap; the app scrolls sideways nowhere.** The document pane's tab row wraps onto a second line instead of scrolling. That retires the one sanctioned horizontal scroller, so the no-horizontal-scroll rule now has no exceptions.

Also settled with these amendments:
- **`label-caps` is set as typed.** Uppercase is applied only to label *words* (TRAY, QUESTION, phase names), never to identifiers such as emails, ids, slugs or model names.
- **The participant palette keeps its eight colour names.** Only the hex values are tuned to the navy theme, because a participant's colour override is stored by name.
