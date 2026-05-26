import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        author: {
          brian: "#f97316",
          rain: "#a855f7",
          emma: "#22c55e",
          user: "#3b82f6",
        },
      },
    },
  },
  plugins: [],
} satisfies Config;
