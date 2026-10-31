// Shared types and HTTP helpers for the network providers.
import type { Metric, Options, Series } from "../market"

export type Tokens = { access: string; refresh?: string; expiresAt?: number; scope?: string }
export type Client = { id: string; secret: string }
export type Profile = { id: string; name: string; avatar?: string; target?: Choice }
export type Choice = { id: string; name: string; token?: string }
export type Media = { url: string; type: string; size: number }
export type Job = { tokens: Tokens; targetId?: string; targetToken?: string; content: string; options: Options; media: Media[] }
export type Sent = { id: string; url?: string; pending?: boolean }
export type Status = { state: "published" | "failed" | "processing"; id?: string; url?: string; reason?: string }
export type Measured = {
  status?: "ready" | "partial" | "target_required" | "permission_required" | "unsupported"
  metrics: Metric[]
  series: Series[]
  note?: string
}
export type Measure = { tokens: Tokens; targetId?: string; targetToken?: string; days: number; from: Date; to: Date; published: string[] }
export type Init = RequestInit & { json?: unknown; form?: Record<string, string>; token?: string; basic?: [string, string]; duplex?: "half" }
/** Sends a request for a publishing job. `publishes` marks the call that makes the post public. */
export type Send = (url: string, init?: Init, publishes?: boolean) => Promise<Response>

export type Provider = {
  /** Environment variables holding the OAuth client ID and secret. */
  env: [string, string]
  pkce: boolean
  authorize(input: { client: Client; redirect: string; state: string; challenge?: string }): string
  exchange(input: { client: Client; redirect: string; code: string; verifier?: string }): Promise<Tokens>
  refresh?(tokens: Tokens, client: Client): Promise<Tokens>
  profile(tokens: Tokens): Promise<Profile>
  /** Pages, organizations or business accounts the connection can post to. */
  targets?(tokens: Tokens): Promise<Choice[]>
  publish(job: Job, send: Send): Promise<Sent>
  status?(job: Job, remoteId: string, send: Send): Promise<Status>
  metrics(input: Measure): Promise<Measured>
}

/** Networks or features still waiting on platform approval, from MARKET_PENDING (for example "reddit,linkedin-page"). */
export function pending(name: string) {
  return (process.env.MARKET_PENDING ?? "")
    .split(",")
    .map((item) => item.trim())
    .includes(name)
}

/** A network answered with an error. `rejected` means the post was definitely not created. */
export class ProviderError extends Error {
  constructor(
    readonly status: number,
    message: string,
    public rejected = false,
  ) {
    super(message)
  }
}

export async function request(url: string, init: Init = {}) {
  const { json, form, token, basic, headers, body, signal, ...rest } = init
  const merged = new Headers(headers)
  if (token) merged.set("Authorization", `Bearer ${token}`)
  if (basic) merged.set("Authorization", `Basic ${btoa(basic.join(":"))}`)
  if (json !== undefined) merged.set("Content-Type", "application/json")
  const response = await fetch(url, {
    ...rest,
    headers: merged,
    body: json !== undefined ? JSON.stringify(json) : form ? new URLSearchParams(form) : body,
    signal: signal ?? AbortSignal.timeout(60_000),
  } as RequestInit)
  if (!response.ok) throw new ProviderError(response.status, `${new URL(url).hostname} returned ${response.status}: ${await problem(response)}`)
  return response
}

export async function json(url: string, init?: Init): Promise<any> {
  return await (await request(url, init)).json()
}

export function link(base: string, params: Record<string, string | undefined>) {
  const url = new URL(base)
  for (const [key, value] of Object.entries(params)) if (value !== undefined) url.searchParams.set(key, value)
  return url.toString()
}

/** Standard OAuth token response. Networks that do not rotate refresh tokens keep the previous one. */
export function tokens(data: any, previous?: Tokens): Tokens {
  if (typeof data?.access_token !== "string") throw new ProviderError(400, String(data?.error_description ?? data?.error ?? "The network did not return an access token."))
  return {
    access: data.access_token,
    refresh: data.refresh_token ?? previous?.refresh,
    expiresAt: data.expires_in ? Date.now() + Number(data.expires_in) * 1000 : undefined,
    scope: typeof data.scope === "string" ? data.scope : previous?.scope,
  }
}

/** A byte range of a stored media file, so large videos are never held in memory whole. */
export async function bytes(url: string, start: number, end: number) {
  const response = await fetch(url, { headers: { Range: `bytes=${start}-${end}` }, signal: AbortSignal.timeout(120_000) })
  if (response.status === 206) return new Uint8Array(await response.arrayBuffer())
  if (response.ok && start === 0) return new Uint8Array(await response.arrayBuffer()).slice(0, end + 1)
  throw new Error("The media file could not be read. Upload it again.")
}

export const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

async function problem(response: Response) {
  const text = await response.text().catch(() => "")
  const data = (() => {
    try {
      return JSON.parse(text)
    } catch {
      return null
    }
  })()
  const message =
    data?.error?.message ??
    data?.error_description ??
    data?.detail ??
    data?.errors?.[0]?.message ??
    data?.message ??
    data?.title ??
    (typeof data?.error === "string" ? data.error : null) ??
    (text || response.statusText)
  return String(message).slice(0, 300)
}
