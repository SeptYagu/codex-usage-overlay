/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        overlay: {
          bg: "#1B1D24",
          border: "#4C5362",
          text: "#F4F7FC",
          muted: "#C4CCD8",
          subtle: "#8A94A6",
          green: "#52D273",
          yellow: "#FFC857",
          red: "#FF5C5C",
        }
      },
      fontFamily: {
        mono: ["Consolas", "ui-monospace", "SFMono-Regular", "Menlo", "Monaco", "monospace"],
      }
    },
  },
  plugins: [],
}
