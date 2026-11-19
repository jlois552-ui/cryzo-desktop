import { authTokenFromCredentials } from "@/runtime/server/api"
import type { Platform } from "@/runtime/platform/platform"

/** Calls the local engine's Cryzo Cloud and publishing endpoints for one project folder. */
export async function cloudRequest(input: {
  url: string
  password?: string
  platform: Platform
  directory: string
  path:
    | "cloud"
    | "cloud/link"
    | "cloud/unlink"
    | "cloud/preview"
    | "publish"
    | "apps"
    | "apps/new"
    | "apps/unpublish"
    | "apps/data"
    | "transcribe"
    | "apps/secrets"
    | "apps/secrets/set"
    | "apps/secrets/delete"
  body?: unknown
  fallback: string
}) {
  const response = await (input.platform.fetch ?? fetch)(
    `${input.url}/api/cryzo/${input.path}?location%5Bdirectory%5D=${encodeURIComponent(input.directory)}`,
    {
      method: input.path === "cloud" || input.path === "apps" ? "GET" : "POST",
      headers: {
        ...(input.password ? { Authorization: `Basic ${authTokenFromCredentials({ password: input.password })}` } : {}),
        ...(input.body ? { "Content-Type": "application/json" } : {}),
      },
      body: input.body ? JSON.stringify(input.body) : undefined,
    },
  )
  const data = (await response.json().catch(() => ({}))) as Record<string, unknown>
  if (!response.ok) throw new Error(typeof data.message === "string" ? data.message : input.fallback)
  return data
}
