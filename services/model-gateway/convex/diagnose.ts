import { internalAction } from "./_generated/server"
import { modelIDs } from "./lib/models"

// Operator check, never exposed over HTTP: `bunx convex run diagnose:models`.
// Sends a one-token request per curated model with the owner key and reports only status and latency.
export const models = internalAction({
  args: {},
  handler: async () =>
    Promise.all(modelIDs.map(async (model) => {
      const started = Date.now()
      const response = await fetch("https://integrate.api.nvidia.com/v1/chat/completions", {
        method: "POST",
        headers: { Authorization: `Bearer ${process.env.NVIDIA_API_KEY}`, "Content-Type": "application/json" },
        body: JSON.stringify({ model, messages: [{ role: "user", content: "hi" }], max_tokens: 1 }),
        signal: AbortSignal.timeout(60000),
      }).catch((error: unknown) => (error instanceof Error ? error.name : "error"))
      return { model, status: typeof response === "string" ? response : response.status, ms: Date.now() - started }
    })),
})
