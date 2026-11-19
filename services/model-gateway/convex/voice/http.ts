// The composer's mic: transcribes a short recording with Whisper on Cloudflare Workers AI.
import type { HttpRouter } from "convex/server"
import { internal } from "../_generated/api"
import { httpAction } from "../_generated/server"
import { account } from "../gateway"
import { recording } from "../lib/voice"

const headers = { "Content-Type": "application/json", "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff" }
const reply = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers })
const message = (error: unknown) =>
  error instanceof Error ? error.message.replace(/^.*?Uncaught Error:\s*/, "").split("\n")[0] : "Dictation failed."

export function voiceRoutes(http: HttpRouter) {
  http.route({
    path: "/voice/transcribe",
    method: "POST",
    handler: httpAction(async (ctx, request) => {
      const user = await account(ctx, request)
      if (!user) return reply({ error: "Sign in to Cryzo to use dictation.", code: "sign_in" }, 401)
      const input = (await request.json().catch(() => null)) as { audio?: unknown } | null
      const checked = recording(input?.audio)
      if ("problem" in checked) return reply({ error: checked.problem }, 400)
      try {
        await ctx.runMutation(internal.voice.store.spend, {
          user,
          day: new Date().toISOString().slice(0, 10),
          seconds: checked.seconds,
        })
      } catch (error) {
        return reply({ error: message(error) }, 429)
      }
      try {
        const text = await ctx.runAction(internal.voice.whisper.transcribe, { audio: input?.audio as string })
        return reply({ data: { text } })
      } catch (error) {
        return reply({ error: message(error) }, 502)
      }
    }),
  })
}
