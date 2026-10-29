export const modelIDs = [
  "deepseek-ai/deepseek-v4.1-flash", "z-ai/glm-5.3-flash", "minimaxai/minimax-m3.1-flash",
  "nvidia/nemotron-3-ultra-550b-a55b", "nvidia/nemotron-3.5-lightning-30b-a3b", "nvidia/nemotron-3.5-super-vl-120b-a12b",
] as const

export function availableModels(data: unknown) {
  if (!data || typeof data !== "object" || !("data" in data) || !Array.isArray(data.data)) throw new Error("invalid NVIDIA catalog")
  return data.data.flatMap((model: unknown) => {
    if (!model || typeof model !== "object" || !("id" in model) || typeof model.id !== "string") return []
    return modelIDs.some((id) => id === model.id) ? [{ id: model.id, object: "model", owned_by: "cryzo" }] : []
  })
}

export function inferenceBody(input: unknown) {
  if (!input || typeof input !== "object" || !("model" in input) || !modelIDs.some((id) => id === input.model)) throw new Error("unsupported free model")
  if (!("messages" in input) || !Array.isArray(input.messages) || input.messages.length < 1 || input.messages.length > 512) throw new Error("invalid messages")
  if (!("cryzo_step_id" in input) || typeof input.cryzo_step_id !== "string" || !/^msg_[a-zA-Z0-9_-]{1,120}$/.test(input.cryzo_step_id)) throw new Error("invalid step")
  const body: Record<string, unknown> = { model: input.model, messages: input.messages, stream: true, stream_options: { include_usage: true }, max_tokens: 16384 }
  // Explicit allowlist prevents provider routing, arbitrary URLs and paid fallbacks.
  for (const key of ["tools", "tool_choice", "reasoning_effort", "chat_template_kwargs"] as const) {
    if (key in input) body[key] = Reflect.get(input, key)
  }
  return { step: input.cryzo_step_id, body }
}
