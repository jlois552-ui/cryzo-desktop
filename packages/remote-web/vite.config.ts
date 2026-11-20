import { defineConfig } from "vite"
import app from "../app/vite.js"
import { brandAlias, branding } from "../desktop-tauri/branding"

// The Cryzo web app for phones and browsers, carried to the user's computer by the remote relay.
// Build with VITE_CRYZO_GATEWAY_URL set to the Convex gateway's site URL.
export default defineConfig({
  plugins: [
    branding,
    app,
  ],
  publicDir: "../app/public",
  resolve: { alias: brandAlias, dedupe: ["solid-js", "@solidjs/router", "effect", "@tanstack/solid-query"] },
  define: {
    "import.meta.env.OPENCODE_VERSION": JSON.stringify("0.1.0"),
    "import.meta.env.OPENCODE_TEST_ONBOARDING": JSON.stringify(false),
  },
  build: { target: "es2022" },
})
