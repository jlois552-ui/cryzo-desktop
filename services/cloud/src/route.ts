// Pure routing and path rules for the cryzo-cloud Worker, kept apart from the Durable Object so
// they can be tested without the Workers runtime.

export const ROOT = "/workspace"

const name = "[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?"

export type WorkspaceRoute = {
  workspace: string
  action: "exec" | "read" | "write" | "list" | "delete" | "hashes" | "pty" | "stop"
}

const actions: Record<string, WorkspaceRoute["action"]> = {
  exec: "exec",
  "fs/read": "read",
  "fs/write": "write",
  "fs/list": "list",
  "fs/delete": "delete",
  "fs/hashes": "hashes",
  pty: "pty",
  stop: "stop",
}

/** `/w/<workspace>/<action>` for the desktop engine's workspace API. */
export function workspaceRoute(pathname: string): WorkspaceRoute | null {
  const match = new RegExp(`^/w/(${name})/(.+)$`).exec(pathname)
  const action = match ? actions[match[2]!] : undefined
  return match && action ? { workspace: match[1]!, action } : null
}

/**
 * `<port>-<workspace>.preview.<app domain>`: a dev server inside a workspace, on a hostname of its
 * own so a preview never shares cookies with Cryzo or another workspace.
 */
export function previewHost(host: string, domain: string) {
  const suffix = `.preview.${domain}`
  if (!host.endsWith(suffix)) return null
  const match = new RegExp(`^([0-9]{2,5})-(${name})$`).exec(host.slice(0, -suffix.length))
  if (!match) return null
  const port = Number(match[1])
  return port >= 1024 && port <= 65535 ? { port, workspace: match[2]! } : null
}

/** Folders that never sync between the desktop and the workspace: dependencies, git and build output. */
export const SKIPPED = ["node_modules", ".git", ".cryzo", ".next", ".svelte-kit", "target", ".turbo"]

/** `sha256sum` lines (`<hash>  ./<path>`) as a map from project-relative path to hash. */
export function parseHashes(output: string) {
  const hashes: Record<string, string> = {}
  for (const line of output.split("\n")) {
    const match = /^([0-9a-f]{64}) [ *]\.\/(.+)$/.exec(line)
    if (match) hashes[match[2]!] = match[1]!
  }
  return hashes
}

/** An absolute path inside the workspace, or null when the path would leave it. */
export function inside(path: string) {
  if (typeof path !== "string" || path.includes("\0")) return null
  const parts: string[] = []
  const relative = path.startsWith(`${ROOT}/`) ? path.slice(ROOT.length + 1) : path === ROOT ? "" : path
  if (relative.startsWith("/")) return null
  for (const part of relative.split("/")) {
    if (part === "" || part === ".") continue
    if (part === "..") {
      if (!parts.length) return null
      parts.pop()
      continue
    }
    parts.push(part)
  }
  return parts.length ? `${ROOT}/${parts.join("/")}` : ROOT
}
