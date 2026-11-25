import { mkdir, mkdtemp } from "node:fs/promises"
import path from "node:path"
import { tasks } from "./tasks"
import { type Action, type Backend, type Element, type Window } from "./faceoff-backends"

const programs = {
  calculator: { command: "calc.exe", processes: ["Calculator", "CalculatorApp"], app: /^(calculator|calculatorapp|applicationframehost)\.exe$/i, title: /^Calculator$/i },
  "notepad-new": { command: "notepad.exe", processes: ["notepad"], app: /^notepad\.exe$/i, title: /notepad/i },
  "notepad-edit": { command: "notepad.exe", processes: ["notepad"], app: /^notepad\.exe$/i, title: /notepad/i },
  explorer: { command: "explorer.exe", processes: [], app: /^explorer\.exe$/i, title: /^inbox$/i },
  settings: { command: "ms-settings:about", processes: ["SystemSettings"], app: /^(systemsettings|applicationframehost)\.exe$/i, title: /^Settings$/i },
}
type TaskId = keyof typeof programs
type Options = {
  work: string
  runs: number
  modelBridge: string
  oauthDb: string
  cryzo(): Backend
  vercel(): Backend
}
type Trace = { action: string; ms: number; ok: boolean; detail?: string }
type Row = {
  backend: string; task: TaskId; run: number; ok: boolean; seconds: number; detail: string
  timeout: boolean; providerError: boolean; blockedTargets: number; retries: number
  input: number; output: number; trace: Trace[]
}
type Call = { id: string; type: "function"; function: { name: string; arguments: string } }
type Reply = { text: string; native: unknown; calls: Call[]; usage: { prompt_tokens?: number; completion_tokens?: number } }
const deadlineMs = 150_000
const maxSteps = 26
const keys = new Set([
  "ctrl+a", "ctrl+c", "ctrl+x", "ctrl+v", "ctrl+s", "ctrl+shift+s", "ctrl+shift+n", "ctrl+w", "ctrl+l",
  "return", "escape", "tab", "shift+tab", "f2", "up", "down", "left", "right", "space",
])
const tools = [{
  type: "function",
  function: {
    name: "desktop",
    description: "Operate the test app by explicit window ID. List windows first. Snapshot or find returns current element references. Use press for supported shortcuts, click for buttons and file selection, and set_text for editable fields. Snapshot again after acting. A save dialog may have a new window ID.",
    parameters: {
      type: "object",
      properties: {
        action: { type: "string", enum: ["windows", "snapshot", "find", "click", "scroll", "set_text", "press"] },
        window: { type: "string", description: "ID from windows, required except for windows" },
        ref: { type: "string", description: "Current element reference for click, scroll or set_text" },
        value: { type: "string", description: "Search string, text to enter, or one key chord such as ctrl+s" },
      },
      required: ["action"],
      additionalProperties: false,
    },
  },
}]

function bridge(binary: string, oauthDb: string, scratchDb: string) {
  const child = Bun.spawn([binary, oauthDb, scratchDb], { stdin: "pipe", stdout: "pipe", stderr: "ignore" })
  const reader = child.stdout.pipeThrough(new TextDecoderStream()).getReader()
  let buffer = ""
  async function read(timeout: number) {
    const timer = setTimeout(() => child.kill(), timeout)
    try {
      while (!buffer.includes("\n")) {
        const chunk = await reader.read()
        if (chunk.done) throw new Error("The shared model driver stopped. Check the dedicated profile sign-in and deadline.")
        buffer += chunk.value
      }
      const end = buffer.indexOf("\n")
      const reply = JSON.parse(buffer.slice(0, end))
      buffer = buffer.slice(end + 1)
      return reply
    } finally {
      clearTimeout(timer)
    }
  }
  return {
    async ready() {
      if (!(await read(15_000)).ready) throw new Error("Model driver did not start")
    },
    async prompt(messages: object[], timeout: number): Promise<Reply> {
      child.stdin.write(JSON.stringify({ messages, tools }) + "\n")
      await child.stdin.flush()
      const response = await read(timeout)
      if (response.error) throw new Error(String(response.error))
      return response.result
    },
    async stop() {
      child.kill()
      await child.exited
    },
    stopped() {
      return child.exitCode !== null
    },
  }
}

const taskIds = Object.keys(programs) as TaskId[]
export async function nativeMatrix(options: Options) {
  const rows: Row[] = []
  const model = bridge(options.modelBridge, options.oauthDb, path.join(options.work, "model.sqlite"))
  try {
    await model.ready()
    matrix: for (const [taskIndex, id] of taskIds.entries()) {
      for (let run = 1; run <= options.runs; run++) {
        const order = (run + taskIndex) % 2 ? [options.cryzo, options.vercel] : [options.vercel, options.cryzo]
        for (const make of order) {
          const backend = make()
          try {
            const row = await attempt(backend, model, options.work, id, run)
            rows.push(row)
            console.log(row.backend, id, run, row.ok ? "PASS" : "FAIL", row.detail)
            await Bun.write(path.join(options.work, "native.json"), JSON.stringify(rows, null, 2))
            if (model.stopped() || row.providerError) {
              console.log("Matrix stopped after a model driver or provider failure. Remaining trials are unrun.")
              break matrix
            }
          } finally {
            await backend.stop()
          }
        }
      }
    }
  } finally {
    await model.stop()
    const report = summary(rows)
    console.log(report)
    await Bun.write(path.join(options.work, "native.md"), report + "\n")
  }
  if (rows.length !== taskIds.length * options.runs * 2) {
    throw new Error("The native task matrix is incomplete. See native.md and native.json in the report directory.")
  }
}

async function attempt(backend: Backend, model: ReturnType<typeof bridge>, work: string, id: TaskId, run: number): Promise<Row> {
  const task = tasks.find((task) => task.id === id)!
  const program = programs[id]
  const dir = await mkdtemp(path.join(work, id + "-"))
  const tag = path.basename(dir).slice(-8)
  const context = { dir, tag, web: "", received: [], downloads: dir }
  await mkdir(dir, { recursive: true })
  await task.setup?.(context)
  const previous = await backend.windows()
  if (previous.some((window) => program.app.test(window.app) && program.title.test(window.title))
    || (await fixtureProcesses(program.processes)).length) {
    throw new Error("Close pre-existing " + id + " windows in the dedicated desktop before running")
  }
  const oldIds = new Set(previous.map((window) => window.id))
  const args = id === "notepad-edit" ? [path.join(dir, "draft-" + tag + ".txt")]
    : id === "explorer" ? [path.join(dir, "inbox")] : []
  const launcher = id === "settings" ? ["explorer.exe", program.command] : [program.command, ...args]
  const launched = Bun.spawn(launcher, { stdout: "ignore", stderr: "ignore" })
  let primaryId: string | undefined
  async function targets() {
    const found = (await backend.windows()).filter((window) => !oldIds.has(window.id) && program.app.test(window.app)
      && (program.title.test(window.title) || /^save as$/i.test(window.title)))
    // Windows can expose both an ApplicationFrameHost window and its child for a single UWP app.
    if (id !== "calculator" && id !== "settings") return found
    if (!primaryId) {
      primaryId = found.sort((left, right) => Number(right.focused) - Number(left.focused) || right.area - left.area)[0]?.id
    }
    return found.filter((window) => window.id === primaryId)
  }
  let windows: Window[] = []
  const refs = new Map<string, Element>()
  const clicked: string[] = []
  const failed = new Set<string>()
  let displayVerified = false
  let started = performance.now()
  let finished: number | undefined
  let owned: { id: number; ticks: string }[] = []
  const row: Row = {
    backend: backend.name, task: id, run, ok: false, seconds: 0, detail: "",
    timeout: false, providerError: false, blockedTargets: 0, retries: 0, input: 0, output: 0, trace: [],
  }
  function blocked(message: string): never {
    row.blockedTargets++
    throw new Error(message)
  }
  async function desktop(call: Call) {
    if (call.function.name !== "desktop") return blocked("Only the desktop benchmark tool is available")
    const input = JSON.parse(call.function.arguments)
    if (input.action === "windows") {
      windows = await targets()
      return windows
    }
    const window = windows.find((candidate) => candidate.id === input.window)
    if (!window || !(await targets()).some((candidate) => candidate.id === window.id)) {
      return blocked("Window is outside this trial or no longer open. List windows again.")
    }
    if (input.action === "snapshot" || input.action === "find") {
      refs.clear()
      const name = input.action === "find" ? String(input.value ?? "").slice(0, 150) : undefined
      const result = (await backend.inspect(window, name)).slice(0, 200)
      for (const element of result) {
        if (element.ref) refs.set(window.id + ":" + element.ref, element)
      }
      if (id === "calculator" && result.some((element) => /(?:^|[^\d])248,?171(?:[^\d]|$)/.test(element.name + " " + (element.value ?? "")))) {
        displayVerified = true
      }
      return result
    }
    const action = input.action as Action
    const value = typeof input.value === "string" ? input.value : undefined
    const element = refs.get(window.id + ":" + input.ref)
    if (action !== "press" && !element) return blocked("Element reference is outside the scoped window or stale")
    if (action === "press") {
      if (id === "calculator" || !value || !keys.has(value)) return blocked("Key is outside the trial policy")
    } else if (action === "set_text") {
      if (id === "calculator" || id === "settings" || !value || value.length > 2048) return blocked("Text is outside the trial policy")
      if (/[A-Z]:[\\/]|\\\\|https?:|(?:^|[\\/])\.\.(?:[\\/]|$)/i.test(value)) {
        const relative = path.relative(dir, value)
        if (!path.isAbsolute(value) || path.isAbsolute(relative) || relative.startsWith("..")) {
          return blocked("File path is outside the disposable fixture")
        }
      }
    } else if (action !== "click" && action !== "scroll") {
      return blocked("Action is outside the trial policy")
    }
    const result = await backend.act(window, action, element?.ref, value)
    if (action === "click" && element) clicked.push(element.name)
    refs.clear()
    return result ?? "Action reported success. Observe the window to verify."
  }
  try {
    for (let attempt = 0; attempt < 20; attempt++) {
      windows = await targets()
      if (windows.length) break
      await Bun.sleep(300)
    }
    owned = await fixtureProcesses(program.processes)
    if (!windows.length) throw new Error("The fixture app did not expose a fresh window")
    const prompt = task.prompt(context).replace(" (the desktop tools)", "")
    const messages: object[] = [
      { role: "developer", content: "Use only the supplied desktop tool. The target is already open. Keep effects inside the fixture. Do not infer task results without using the desktop. Observe after each action and use the latest references. Closing a window with its Close button is supported." },
      { role: "user", content: prompt },
    ]
    started = performance.now()
    let answer = ""
    for (let step = 0; step < maxSteps; step++) {
      const remaining = deadlineMs - (performance.now() - started)
      if (remaining <= 0) break
      let reply: Reply
      try {
        reply = await model.prompt(messages, remaining)
      } catch (error) {
        row.providerError = (performance.now() - started) < deadlineMs
        throw error
      }
      row.input += reply.usage?.prompt_tokens ?? 0
      row.output += reply.usage?.completion_tokens ?? 0
      if (!reply.calls.length) { answer = reply.text; break }
      messages.push({ role: "assistant", content: reply.text, cryzo_native: reply.native, tool_calls: reply.calls })
      for (const call of reply.calls) {
        if (performance.now() - started >= deadlineMs) break
        const callStarted = performance.now()
        const signature = call.function.arguments
        if (failed.has(signature)) row.retries++
        let result: unknown
        try {
          result = await desktop(call)
          row.trace.push({ action: JSON.parse(call.function.arguments).action, ms: performance.now() - callStarted, ok: true })
        } catch (error) {
          failed.add(signature)
          const detail = String(error).slice(0, 240)
          result = { error: detail }
          row.trace.push({ action: call.function.name, ms: performance.now() - callStarted, ok: false, detail })
        }
        messages.push({ role: "tool", tool_call_id: call.id, content: JSON.stringify(result) })
      }
    }
    finished = performance.now()
    const checked = await task.check(context, answer)
    const calculator = id !== "calculator" || (displayVerified && calculatorButtons(clicked))
    const remaining = await targets()
    const closed = id === "explorer"
      || (id.startsWith("notepad") ? !remaining.some((window) => window.title.includes(tag)) : remaining.length === 0)
    row.ok = checked.ok && calculator && closed && !!answer && row.blockedTargets === 0
    row.detail = row.blockedTargets ? "An action was blocked by the trial policy"
      : !calculator ? "Missing verified Calculator button sequence and display"
      : !closed ? "The task window or document remained open" : checked.detail
  } catch (error) {
    row.detail = String(error).slice(0, 300)
  } finally {
    row.seconds = ((finished ?? performance.now()) - started) / 1000
    row.timeout = row.seconds >= deadlineMs / 1000
    if (row.timeout) row.ok = false
    if (id === "explorer") await task.cleanup?.(context)
    await resetProcesses(owned)
    if (id !== "explorer" && id !== "settings") launched.kill()
  }
  return row
}

async function fixtureProcesses(names: readonly string[]): Promise<{ id: number; ticks: string }[]> {
  if (!names.length) return []
  const script = "$s=(Get-Process -Id $PID).SessionId; @(Get-Process -Name " + names.join(",")
    + " -ErrorAction SilentlyContinue | Where-Object SessionId -eq $s | ForEach-Object { @{ id=$_.Id; ticks=$_.StartTime.Ticks.ToString() } }) | ConvertTo-Json -Compress"
  const child = Bun.spawn(["powershell.exe", "-NoProfile", "-Command", script], { stdout: "pipe", stderr: "ignore" })
  const text = (await new Response(child.stdout).text()).trim()
  if (await child.exited) throw new Error("Could not check fixture processes")
  const parsed = text ? JSON.parse(text) : []
  return Array.isArray(parsed) ? parsed : [parsed]
}

async function resetProcesses(owned: { id: number; ticks: string }[]) {
  for (const item of owned) {
    if (!Number.isInteger(item.id) || !/^\d+$/.test(item.ticks)) continue
    const script = "$p=Get-Process -Id " + item.id + " -ErrorAction SilentlyContinue;"
      + " if ($p -and $p.StartTime.Ticks.ToString() -eq '" + item.ticks + "') { Stop-Process -InputObject $p -ErrorAction SilentlyContinue }"
    await Bun.spawn(["powershell.exe", "-NoProfile", "-Command", script], { stdout: "ignore", stderr: "ignore" }).exited
  }
}

export function calculatorButtons(labels: string[]) {
  const aliases: Record<string, string> = { eight: "8", four: "4", seven: "7", two: "2", nine: "9", three: "3",
    multiply: "*", "multiply by": "*", "×": "*", equal: "=", equals: "=", "equal to": "=" }
  const significant = labels.map((label) => {
    const key = label.trim().toLowerCase()
    return aliases[key] ?? (/^[0-9*=]$/.test(key) ? key : "")
  }).join("")
  return significant.includes("847*293=")
}

function summary(rows: Row[]) {
  const header = [
    "# Native Windows task matrix", "",
    "Both backends use GPT-6.1 Sol High. Vercel uses headed mode here so physical text and keys are available.", "",
    "| Backend | Task | Passed / attempted | Median seconds on passes | Timeouts | Provider failures | Blocked targets | Calls | Tool errors | Retries | Input / output tokens |",
    "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|",
  ]
  for (const [key, attempts] of Map.groupBy(rows, (row) => row.backend + "|" + row.task)) {
    const times = attempts.filter((row) => row.ok).map((row) => row.seconds).sort((a, b) => a - b)
    const median = times.length ? ((times[Math.floor((times.length - 1) / 2)] + times[Math.floor(times.length / 2)]) / 2).toFixed(1) : "—"
    const sum = (field: "input" | "output" | "blockedTargets" | "retries") => attempts.reduce((total, row) => total + row[field], 0)
    const trace = attempts.flatMap((row) => row.trace)
    header.push("| " + [key.replace("|", " | "), times.length + "/" + attempts.length, median,
      attempts.filter((row) => row.timeout).length, attempts.filter((row) => row.providerError).length,
      sum("blockedTargets"), trace.length, trace.filter((call) => !call.ok).length, sum("retries"), sum("input") + " / " + sum("output")].join(" | ") + " |")
  }
  return header.join("\n")
}
