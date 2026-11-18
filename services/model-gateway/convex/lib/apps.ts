// Published apps: each one is its own Cloudflare Worker with static assets and its own D1
// database, served at `<slug>.<app domain>`. Like Replit's free tier, a user may keep a small
// number of apps live; republishing an app they own never counts against that.

export const FREE_APPS = 1

const RESERVED = new Set(["www", "api", "app", "apps", "admin", "auth", "mail", "preview", "cryzo", "status", "docs", "help", "support"])

/** Why a requested address can't be used, or undefined when it can. */
export function slugProblem(slug: string) {
  if (slug.length < 3 || slug.length > 40) return "Use 3 to 40 characters."
  if (!/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/.test(slug)) return "Use lowercase letters, numbers and hyphens, starting and ending with a letter or number."
  if (slug.includes("--")) return "Don't use two hyphens in a row."
  if (RESERVED.has(slug)) return "That address is reserved."
  return undefined
}

export const scriptName = (slug: string) => `app-${slug}`

/**
 * The Worker module for apps that ship only static files: it serves the build output, with
 * unknown paths falling back to index.html for client-side routing.
 */
export const STATIC_MODULE = `export default { fetch(request, env) { return env.ASSETS.fetch(request) } }\n`

export type Manifest = Record<string, { hash: string; size: number }>

const MAX_FILES = 20_000
const MAX_FILE_BYTES = 25 * 1024 * 1024

/** Checks a build manifest from the desktop before it is sent to Cloudflare. */
export function manifestProblem(manifest: unknown): string | undefined {
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) return "The build manifest is missing."
  const entries = Object.entries(manifest)
  if (entries.length === 0) return "The build folder is empty."
  if (entries.length > MAX_FILES) return `Apps can have at most ${MAX_FILES} files.`
  const bad = entries.find(
    ([path, file]) =>
      !path.startsWith("/") ||
      path.includes("..") ||
      !file ||
      typeof file !== "object" ||
      typeof file.hash !== "string" ||
      !/^[0-9a-f]{32}$/.test(file.hash) ||
      typeof file.size !== "number" ||
      file.size < 0 ||
      file.size > MAX_FILE_BYTES,
  )
  if (bad) return `The build file ${bad[0]} can't be published (files must be under 25 MB).`
  return undefined
}

export type Module = { name: string; content: string }

// Cloudflare allows 10 MB of compressed Worker code on the paid plan; 10 MB uncompressed stays
// within that and within what one request to the gateway can carry.
const MAX_MODULES = 200
const MAX_CODE_BYTES = 10 * 1024 * 1024

/** The app's server code from the desktop, main module first; none for a static site. */
export function readModules(value: unknown): Module[] {
  if (value === undefined || value === null) return []
  if (!Array.isArray(value) || value.length > MAX_MODULES) throw new Error(`An app can have at most ${MAX_MODULES} server files.`)
  const modules = value.map((module: unknown) => {
    const candidate = (module ?? {}) as Record<string, unknown>
    const name = candidate.name
    const content = candidate.content
    if (typeof name !== "string" || !/^[A-Za-z0-9_][A-Za-z0-9_./-]*\.m?js$/.test(name) || name.includes(".."))
      throw new Error("The app's server code has a file with an unusable name.")
    if (typeof content !== "string") throw new Error(`The server file ${name} is empty.`)
    return { name, content }
  })
  if (new Set(modules.map((module) => module.name)).size !== modules.length) throw new Error("The app's server code names a file twice.")
  if (modules.reduce((total, module) => total + module.content.length, 0) > MAX_CODE_BYTES) throw new Error("The app's server code is too large.")
  return modules
}

export type AssetRouting = {
  not_found_handling: "single-page-application" | "404-page" | "none"
  html_handling?: "auto-trailing-slash" | "force-trailing-slash" | "drop-trailing-slash" | "none"
  run_worker_first?: boolean | string[]
}

/** How static files and the Worker share requests, from the app's Wrangler settings. */
export function readAssetRouting(value: unknown): AssetRouting {
  const input = (value && typeof value === "object" ? value : {}) as Record<string, unknown>
  const found = ["single-page-application", "404-page", "none"].find((option) => option === input.not_found_handling)
  const html = ["auto-trailing-slash", "force-trailing-slash", "drop-trailing-slash", "none"].find((option) => option === input.html_handling)
  const first = input.run_worker_first
  const routes =
    typeof first === "boolean" ||
    (Array.isArray(first) &&
      first.length <= 50 &&
      first.every((rule) => typeof rule === "string" && rule.length <= 200 && /^!?\//.test(rule)))
      ? (first as boolean | string[])
      : undefined
  return {
    not_found_handling: (found ?? "single-page-application") as AssetRouting["not_found_handling"],
    ...(html && { html_handling: html as AssetRouting["html_handling"] }),
    ...(routes !== undefined && { run_worker_first: routes }),
  }
}

// Set by Cryzo on every publish; the user can't replace or remove them.
const MANAGED = new Set(["BETTER_AUTH_SECRET", "APP_URL", "DB", "ASSETS"])
// Cloudflare's limit for one Worker secret.
const MAX_SECRET_BYTES = 5 * 1024

export const managedSecret = (name: string) => MANAGED.has(name)

/** Why a secret can't be saved under this name and value, or undefined when it can. */
export function secretProblem(name: string, value?: string) {
  if (!/^[A-Z][A-Z0-9_]{0,63}$/.test(name)) return "Use capital letters, numbers and underscores, like OPENAI_API_KEY."
  if (MANAGED.has(name)) return `Cryzo sets ${name} itself.`
  if (value === undefined) return undefined
  if (!value) return "Enter the secret's value."
  if (new TextEncoder().encode(value).length > MAX_SECRET_BYTES) return "Secrets can be at most 5 KB."
  return undefined
}

export type Migration = { name: string; hash: string }

/** The migration files the desktop is about to publish: SQL file names with a SHA-256 hash. */
export function readMigrations(value: unknown): Migration[] {
  if (value === undefined || value === null) return []
  if (!Array.isArray(value) || value.length > 1000) throw new Error("The app has too many migrations.")
  return value.map((migration: unknown) => {
    const candidate = (migration ?? {}) as Record<string, unknown>
    if (typeof candidate.name !== "string" || !/^[\w.-]{1,200}\.sql$/.test(candidate.name)) throw new Error("A migration has an unusable name.")
    if (typeof candidate.hash !== "string" || !/^[0-9a-f]{64}$/.test(candidate.hash)) throw new Error("A migration has an unusable hash.")
    return { name: candidate.name, hash: candidate.hash }
  })
}

export type Storage = { database: string; files?: string }

/** Which bindings the app's code expects for its database and files, from its Wrangler settings. */
export function readStorage(value: unknown): Storage {
  const input = (value && typeof value === "object" ? value : {}) as Record<string, unknown>
  const binding = (name: unknown) => (typeof name === "string" && /^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(name) ? name : undefined)
  const files = binding(input.files)
  return { database: binding(input.database) ?? "DB", ...(files && { files }) }
}

export const bucketName = (slug: string) => `app-${slug}`
