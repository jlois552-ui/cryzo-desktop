import path from "node:path"

export type Window = { id: string; title: string; app: string; focused: boolean; area: number }
export type Element = { ref?: string; role: string; name: string; value?: string }
export type Action = "click" | "check" | "scroll" | "set_text" | "press"
export interface Backend {
  name: string
  windows(): Promise<Window[]>
  inspect(window: Window, name?: string): Promise<Element[]>
  act(window: Window, action: Action, ref?: string, value?: string, waitFor?: string): Promise<void>
  stop(): Promise<void>
}

const replyTimeout = 12_000

function openMcp(command: string[], env: typeof process.env) {
  const process = Bun.spawn(command, {
    stdin: "pipe",
    stdout: "pipe",
    stderr: env.CRYZO_DESKTOP_PROFILE_SETTLE && env.CRYZO_DESKTOP_CONFIG ? "inherit" : "ignore",
    env,
  })
  const reader = process.stdout.pipeThrough(new TextDecoderStream()).getReader()
  let remaining = ""
  let id = 0
  async function request(method: string, params: object) {
    const requestId = ++id
    process.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: requestId, method, params }) + "\n")
    await process.stdin.flush()
    const timer = setTimeout(() => process.kill(), replyTimeout)
    try {
      while (!remaining.includes("\n")) {
        const chunk = await reader.read()
        if (chunk.done) throw new Error("Desktop MCP stopped or exceeded its deadline")
        remaining += chunk.value
      }
      const newline = remaining.indexOf("\n")
      const reply = JSON.parse(remaining.slice(0, newline))
      remaining = remaining.slice(newline + 1)
      if (reply.id !== requestId) throw new Error("Unexpected desktop MCP reply")
      if (reply.error) throw new Error(reply.error.message ?? "Desktop MCP request failed")
      return reply.result
    } finally {
      clearTimeout(timer)
    }
  }
  const ready = request("initialize", {
    protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "cryzo-eval", version: "1" },
  }).then(() => {
    process.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) + "\n")
  })
  // The fixture may fail before the first tool call waits for the handshake.
  void ready.catch(() => undefined)
  return {
    async call(name: string, args: object = {}) {
      await ready
      return request("tools/call", { name, arguments: args })
    },
    async stop() {
      process.kill()
      await process.exited
      await ready.catch(() => undefined)
    },
  }
}

export function cryzo(binary: string, directory: string): Backend {
  const mcp = openMcp([binary], { ...process.env, CRYZO_DESKTOP_CONFIG: path.join(directory, "cryzo.json") })
  async function call(name: string, args?: object): Promise<string> {
    const result = await mcp.call(name, args)
    const text = result?.content?.[0]?.text
    if (result?.isError || typeof text !== "string") throw new Error(text ?? "No Cryzo tool result")
    return text
  }
  return {
    name: "cryzo",
    async windows() {
      return (await call("windows")).split("\n").flatMap((line) => {
        const match = /^\[(w[0-9a-f]+)\] ("(?:\\.|[^"])*") \(([^)]+)\) (\d+)x(\d+)( \*focused)?/i.exec(line)
        return match ? [{ id: match[1], title: JSON.parse(match[2]), app: match[3],
          area: Number(match[4]) * Number(match[5]), focused: !!match[6] }] : []
      })
    },
    async inspect(window, name) {
      const text = name ? await call("find", { window: window.id, text: name }) : await call("snapshot", { window: window.id })
      return text.split("\n").flatMap((line) => {
        const match = /\[(e\d+)\]\s+(\w+)\s+("(?:\\.|[^"])*")(?:\s+value=("(?:\\.|[^"])*"))?/.exec(line)
        return match ? [{ ref: match[1], role: match[2], name: JSON.parse(match[3]), value: match[4] && JSON.parse(match[4]) }] : []
      })
    },
    async act(window, action, ref, value, waitFor) {
      if (action === "press") {
        const keys = value!.split("+").map((key) => key.length === 1 ? key : "{" + (key === "return" ? "enter" : key) + "}").join("")
        await call("key", { window: window.id, keys })
        return
      }
      const verb = action === "set_text" ? "type" : action === "check" ? "toggle" : action === "scroll" ? "scroll_down" : action
      const response = await call("act", { window: window.id, id: ref, action: verb, value, waitFor })
      if (waitFor && !response.endsWith("Outcome observed.") && !response.includes("\nOutcome observed.\n")) {
        throw new Error("Cryzo did not observe " + waitFor)
      }
    },
    stop: mcp.stop,
  }
}

type VercelNode = {
  ref_id?: string
  role?: string
  name?: string
  value?: unknown
  children?: VercelNode[]
}

export function vercel(binary: string, directory: string, headed: boolean, persistent = false): Backend {
  const env: typeof process.env = { ...process.env, AGENT_DESKTOP_HOME: path.join(directory, "agent-desktop") }
  delete env.AGENT_DESKTOP_SESSION
  delete env.AGENT_DESKTOP_AGENT_ID
  const mode = headed ? ["--headed"] : []
  const mcp = persistent ? openMcp([binary, ...mode, "mcp"], env) : undefined
  function result<T>(reply: { ok: boolean; data: T; error?: { code?: string; message?: string } }): T {
    if (!reply.ok) {
      throw new Error([reply.error?.code, reply.error?.message].filter(Boolean).join(": ") || "Vercel tool failed")
    }
    return reply.data
  }
  async function call<T>(args: string[], params: object = {}, input?: string): Promise<T> {
    if (mcp) {
      const response = await mcp.call("desktop_" + args[0].replaceAll("-", "_"), params)
      const payload = response?.structuredContent
      if (!payload) throw new Error(response?.content?.[0]?.text ?? "No Vercel tool result")
      return result<T>(payload)
    }
    const child = Bun.spawn([binary, ...mode, ...args], {
      env,
      stdin: "pipe",
      stdout: "pipe",
      stderr: "ignore",
    })
    if (input !== undefined) child.stdin.write(input)
    child.stdin.end()
    const timer = setTimeout(() => child.kill(), replyTimeout)
    try {
      const [stdout, exit] = await Promise.all([new Response(child.stdout).text(), child.exited])
      if (!stdout) throw new Error("Vercel CLI exited without a result (" + exit + ")")
      return result<T>(JSON.parse(stdout))
    } finally {
      clearTimeout(timer)
    }
  }
  function elements(nodes: VercelNode[]): Element[] {
    return nodes.flatMap((node) => [
      { ref: node.ref_id, role: node.role ?? "", name: node.name ?? "", value: typeof node.value === "string" ? node.value : undefined },
      ...elements(node.children ?? []),
    ])
  }
  return {
    name: headed ? "vercel-headed" : "vercel",
    async windows() {
      const list = await call<{
        id: string; title: string; app_name: string; is_focused: boolean; bounds?: { width: number; height: number }
      }[]>(["list-windows"])
      return list.map((item) => ({ id: item.id, title: item.title, app: item.app_name, focused: item.is_focused,
        area: item.bounds ? item.bounds.width * item.bounds.height : 0 }))
    },
    async inspect(window, name) {
      if (name) {
        const data = await call<{ matches: VercelNode[] }>(["find", "--window-id", window.id, "--text", name],
          { window_id: window.id, text: name })
        return elements(data.matches)
      }
      const data = await call<{ tree: VercelNode }>(["snapshot", "--window-id", window.id, "--max-depth", "14"],
        { window_id: window.id, max_depth: 14 })
      return elements([data.tree])
    },
    async act(window, action, ref, value) {
      switch (action) {
        case "press":
          await call(["press", value!, "--window-id", window.id], { combo: value, window_id: window.id })
          break
        case "set_text":
          await call([headed ? "type" : "set-value", ref!, "--stdin"],
            { ref_id: ref, ...(headed ? { text: value } : { value }) }, value)
          break
        case "scroll":
          await call(["scroll", ref!, "--direction", "down", "--amount", "3"], { ref_id: ref, direction: "down", amount: 3 })
          break
        default:
          await call([action, ref!], { ref_id: ref })
      }
    },
    async stop() { await mcp?.stop() },
  }
}
