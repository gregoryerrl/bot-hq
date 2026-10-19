import tailwindConfig from "../../tailwind.config";

const colors = tailwindConfig.theme.extend.colors;

/**
 * The Terminal tab's xterm theme. xterm takes literal colours, not classes, so
 * the values are read from the theme tokens instead of restated as hex (S1e):
 * a retuned token reaches the terminal too.
 */
export const XTERM_THEME = {
  background: colors.surface,
  foreground: colors["on-surface"],
  cursor: colors["on-surface"],
  selectionBackground: colors["surface-container-highest"],
};
