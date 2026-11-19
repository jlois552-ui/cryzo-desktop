// The Whisper call itself, separate from the HTTP route so it can be checked on its own.
import { v } from "convex/values"
import { internalAction } from "../_generated/server"

const MODEL = "@cf/openai/whisper-large-v3-turbo"

/** Text spoken in a base64 WAV recording. */
export const transcribe = internalAction({
  args: { audio: v.string() },
  handler: async (_ctx, args): Promise<string> => {
    const account = process.env.CLOUDFLARE_ACCOUNT_ID?.trim()
    const token = process.env.CLOUDFLARE_API_TOKEN?.trim()
    if (!account || !token) throw new Error("Dictation isn't set up on the Cryzo cloud yet.")
    const response = await fetch(`https://api.cloudflare.com/client/v4/accounts/${account}/ai/run/${MODEL}`, {
      method: "POST",
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      body: JSON.stringify({ audio: args.audio }),
      signal: AbortSignal.timeout(60_000),
    })
    const body = (await response.json().catch(() => null)) as {
      success?: boolean
      result?: { text?: string }
      errors?: { message?: string }[]
    } | null
    if (!response.ok || !body?.success)
      throw new Error(`Transcription failed: ${body?.errors?.[0]?.message ?? response.status}`)
    return (body.result?.text ?? "").trim()
  },
})
