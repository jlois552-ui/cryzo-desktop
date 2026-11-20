import { fileURLToPath } from "node:url"
import type { Plugin } from "vite"

/**
 * Cryzo's branding over the upstream app, shared by the desktop build and the remote-control web
 * app so both show the same names, wordmark and logo.
 */
export const branding: Plugin = {
  name: "cryzo-branding",
  enforce: "pre",
  transform(source, id) {
    const path = id.replaceAll("\\", "/").split("?")[0]
    if (path.endsWith("/src/app.tsx")) {
      return source.replace("<Title>OpenCode</Title>", "<Title>Cryzo</Title>")
    }
    if (path.endsWith("/session/route.tsx")) return source.replace(" · OpenCode`", " · Cryzo`")
    if (path.endsWith("/i18n/desktop-native.ts")) return source.replace('"OpenCode menu"', '"Cryzo menu"')
    // Match the released OpenCode desktop's provider wording rather than the upstream branch's integrations.
    if (path.endsWith("/i18n/en.ts"))
      return source
        .replace('"command.provider.connect": "Connect an integration"', '"command.provider.connect": "Connect provider"')
        .replace(
          '"Sign in to OpenCode Go, OpenCode Console, or another model provider"',
          '"Sign in with an API key or account to use its models"',
        )
        .replace('"dialog.provider.group.services": "Services"', '"dialog.provider.group.services": "Other"')
        .replace("models in OpenCode.", "models in Cryzo.")
        .replace("Eligible requests in OpenCode can use your ChatGPT plan.", "Eligible requests in Cryzo can use your ChatGPT plan.")
    if (path.endsWith("/new-session/wordmark.tsx"))
      return source.replace(
        /const LETTERS = \[[\s\S]*?\] as const/,
        `const LETTERS = ${JSON.stringify([
          { cx: 21, weak: "", fill: "var(--icon-base)", stem: "M42 6H0V36H42V30H6V12H42Z" },
          { cx: 69, weak: "", fill: "var(--icon-base)", stem: "M48 6H90V12H54V36H48Z" },
          {
            cx: 117,
            weak: "",
            fill: "var(--icon-strong-base)",
            stem: "M96 6H102V18H132V6H138V24H120V36H114V24H96Z",
          },
          {
            cx: 165,
            weak: "",
            fill: "var(--icon-strong-base)",
            stem: "M144 6H186V12L154 30H186V36H144V30L176 12H144Z",
          },
          // Preserve all eight animation slots; the lodger addresses the final slot directly.
          { cx: 189, weak: "", fill: "var(--icon-base)", stem: "" },
          { cx: 189, weak: "", fill: "var(--icon-base)", stem: "" },
          { cx: 189, weak: "", fill: "var(--icon-base)", stem: "" },
          {
            cx: 213,
            weak: "M228 30H198V18H228Z",
            fill: "var(--icon-strong-base)",
            stem: "M192 6H234V36H192ZM198 12V30H228V12Z",
          },
        ])} as const`,
      )
  },
}

/** The Cryzo logo and wordmark in place of the upstream ones. */
export const brandAlias = {
  "@opencode/ui/logo": fileURLToPath(new URL("./src/brand.tsx", import.meta.url)),
  "@opencode/ui/wordmark": fileURLToPath(new URL("./src/brand.tsx", import.meta.url)),
}
