import { defineConfig } from "vite"
import app from "../app/vite.js"
import { brandAlias, branding } from "./branding"

export default defineConfig({
  plugins: [
    branding,
    app,
  ],
  publicDir: "../app/public",
  resolve: {
    alias: brandAlias,
    dedupe: ["solid-js", "@solidjs/router", "effect", "@tanstack/solid-query"],
  },
  define: {
    "import.meta.env.OPENCODE_VERSION": JSON.stringify("0.1.0"),
    "import.meta.env.OPENCODE_TEST_ONBOARDING": JSON.stringify(false),
  },
  clearScreen: false,
  server: { host: "127.0.0.1", port: 1420, strictPort: true },
  build: { target: "es2022" },
})
