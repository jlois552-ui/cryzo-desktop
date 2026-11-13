// Computer-use evaluation: real tasks driven by a model through the real Cryzo engine.
//
// It starts a separate engine with its own data folder and desktop-control settings (enabled,
// never ask), so the user's Cryzo setup is untouched. Desktop tools run without approval; every
// other permission request (shell, file writes) is rejected, so the model has to finish each task
// through computer use. Results are checked independently of what the model says: file contents,
// what the local web server received, values read from Windows.
//
//   bun native/desktop-control/eval/run.ts --model cryzo:deepseek-ai/deepseek-v4.1-flash --runs 5
//   bun native/desktop-control/eval/run.ts --list-models
//   options: --tasks calculator,form  --runs N  --timeout 300  --engine path\to\cryzo-engine.exe

import { Database } from "bun:sqlite"
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises"
import { existsSync } from "node:fs"
import { tmpdir, homedir } from "node:os"
import path from "node:path"
import { tasks, type Context } from "./tasks"

const args = parseArgs(process.argv.slice(2))
const root = path.resolve(import.meta.dir, "..", "..")
const engineExe = args.engine ?? path.join(root, "target", "release", "cryzo-engine.exe")
if (!existsSync(engineExe)) throw new Error(`engine not found at ${engineExe}; build it with cargo build --release -p cryzo-engine`)

const work = await mkdtemp(path.join(tmpdir(), "cryzo-eval-"))
const project = path.join(work, "project")
await mkdir(project, { recursive: true })
const config = path.join(work, "desktop-control.json")
await writeFile(config, JSON.stringify({ enabled: true, approval: "never", auditLog: path.join(work, "audit.log") }))

// A read-only snapshot of the user's Cryzo data, so the eval engine sees their Cryzo sign-in and
// connected providers; the secrets themselves stay in the OS credential store. Eval sessions land
// only in this copy, which is deleted at the end.
const appData = path.join(process.env.APPDATA ?? "", "me.cryzo.desktop", "cryzo.sqlite")
await mkdir(path.join(work, "data"), { recursive: true })
if (existsSync(appData)) {
  const source = new Database(appData, { readonly: true })
  source.run(`VACUUM INTO '${path.join(work, "data", "cryzo.sqlite").replace(/'/g, "''")}'`)
  source.close()
}

const engine = Bun.spawn([engineExe, "serve", path.join(work, "data")], {
  env: { ...process.env, CRYZO_DESKTOP_CONFIG: config },
  stdout: "pipe",
  stderr: "pipe",
})
const server = await firstLine(engine.stdout)
const api = async (route: string, init: RequestInit = {}) => {
  const response = await fetch(server.url + route, {
    ...init,
    headers: { authorization: `Bearer ${server.token}`, "content-type": "application/json", ...init.headers },
  })
  if (!response.ok) throw new Error(`${init.method ?? "GET"} ${route}: ${response.status} ${await response.text()}`)
  return response.status === 204 ? undefined : response.json()
}

const models = (await api("/api/model")) as { data?: unknown[] } | unknown[]
const modelList = (Array.isArray(models) ? models : (models.data ?? [])) as { id: string; providerID: string; enabled?: boolean; name?: string }[]
if (args["list-models"]) {
  console.log(modelList.map((model) => `${model.providerID}:${model.id}${model.enabled === false ? " (not enabled)" : ""}`).join("\n"))
  await stop()
  process.exit(0)
}
const model = pickModel(args.model)
const selected = args.tasks ? tasks.filter((task) => args.tasks!.split(",").includes(task.id)) : tasks
const runs = Number(args.runs ?? 1)
const timeout = Number(args.timeout ?? 300) * 1000
const fixtures = startFixtures()
const results: Result[] = []

console.log(`model ${model.providerID}:${model.id}, ${selected.length} task(s) × ${runs} run(s), workspace ${work}\n`)
for (const task of selected) {
  for (let run = 1; run <= runs; run++) {
    const dir = path.join(project, `${task.id}-${run}`)
    await mkdir(dir, { recursive: true })
    const context: Context = { dir, web: fixtures.url, received: fixtures.received, downloads: path.join(homedir(), "Downloads") }
    fixtures.received.length = 0
    const result = await attempt(task, context, run)
    results.push(result)
    console.log(
      `${result.ok ? "PASS" : "FAIL"} ${task.id} #${run}  ${result.seconds.toFixed(1)}s  ${result.calls} calls (${result.errors} errors${result.other ? `, ${result.other} non-desktop` : ""})  ${result.detail}`,
    )
  }
}

await writeFile(path.join(work, "results.json"), JSON.stringify({ model, results }, null, 2))
const summary = summarize(results)
await writeFile(path.join(work, "summary.md"), summary)
console.log(`\n${summary}\nresults: ${path.join(work, "results.json")}`)
fixtures.server.stop(true)
await stop()
// Chrome and other children the engine started can keep handles open; the run is over.
process.exit(0)

type Result = {
  task: string
  run: number
  ok: boolean
  provider: boolean
  detail: string
  seconds: number
  calls: number
  errors: number
  other: number
  tokens: { input: number; output: number }
  answer: string
  session: string
  trace: { tool: string; status: string; input: string; error?: string }[]
}

async function attempt(task: (typeof tasks)[number], context: Context, run: number): Promise<Result> {
  await task.setup?.(context)
  const session = (await api("/api/session", {
    method: "POST",
    body: JSON.stringify({ location: { directory: project }, model: { id: model.id, providerID: model.providerID }, title: `eval ${task.id} #${run}` }),
  })) as { data: { id: string } }
  const id = session.data.id
  const started = performance.now()
  await api(`/api/session/${id}/prompt`, { method: "POST", body: JSON.stringify({ text: task.prompt(context) }) })
  const finished = await waitIdle(id, started)
  const seconds = (performance.now() - started) / 1000
  const messages = ((await api(`/api/session/${id}/message?limit=200`)) as { data: Message[] }).data
  const parts = messages.flatMap((message) => message.content ?? [])
  const tools = parts.filter((part) => part.type === "tool")
  const answer = messages
    .filter((message) => message.type === "assistant")
    .flatMap((message) => (message.content ?? []).filter((part) => part.type === "text").map((part) => part.text ?? ""))
    .join("\n")
    .trim()
  const info = ((await api(`/api/session/${id}`)) as { data: { tokens?: { input?: number; output?: number } } }).data
  const failure = messages.map((message) => message.error).find(Boolean)
  const checked = finished ? await task.check(context, answer).catch((error: Error) => ({ ok: false, detail: `check failed: ${error.message}` })) : { ok: false, detail: `timed out after ${timeout / 1000}s` }
  const verdict = failure && !checked.ok ? { ok: false, detail: `model step failed: ${JSON.stringify(failure).slice(0, 300)}` } : checked
  await task.cleanup?.(context)
  return {
    task: task.id,
    run,
    ok: verdict.ok,
    // The model service failing (down, 502, unreachable) says nothing about computer use.
    provider: !verdict.ok && JSON.stringify(failure ?? "").includes("ProviderError"),
    detail: verdict.detail,
    seconds,
    calls: tools.length,
    errors: tools.filter((part) => part.state?.status === "error").length,
    other: tools.filter((part) => !part.name?.startsWith("desktop_")).length,
    tokens: { input: info.tokens?.input ?? 0, output: info.tokens?.output ?? 0 },
    answer: answer.slice(0, 2000),
    session: id,
    trace: tools.map((part) => ({
      tool: part.name ?? "?",
      status: part.state?.status ?? "?",
      input: JSON.stringify(part.state?.input ?? "").slice(0, 200),
      error: part.state?.error === undefined ? undefined : JSON.stringify(part.state.error).slice(0, 300),
    })),
  }
}

type Message = {
  type?: string
  error?: unknown
  content?: { type: string; text?: string; name?: string; state?: { status?: string; input?: unknown; error?: unknown } }[]
}

/** Polls until the run ends, rejecting every permission request: desktop tools never ask here. */
async function waitIdle(id: string, started: number) {
  const deadline = started + timeout
  while (performance.now() < deadline) {
    await Bun.sleep(1000)
    const pending = (await api(`/api/session/${id}/permission`)) as { data?: { id: string }[] } | { id: string }[]
    for (const request of Array.isArray(pending) ? pending : (pending.data ?? [])) {
      await api(`/api/session/${id}/permission/${request.id}/reply`, { method: "POST", body: JSON.stringify({ decision: "reject" }) })
    }
    const active = (await api("/api/session/active")) as { data: Record<string, unknown> }
    if (!(id in active.data) && performance.now() - started > 3000) return true
  }
  await api(`/api/session/${id}/interrupt`, { method: "POST" })
  return false
}

function startFixtures() {
  const received: Record<string, string>[] = []
  const page = (title: string, body: string) =>
    new Response(`<!doctype html><html><head><meta charset="utf-8"><title>${title}</title></head><body style="font-family:sans-serif;margin:40px">${body}</body></html>`, {
      headers: { "content-type": "text/html; charset=utf-8" },
    })
  const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    async fetch(request) {
      const url = new URL(request.url)
      if (url.pathname === "/prices") {
        const rows = [
          ["Desk Chair", "$129.00"],
          ["Blue Lamp", "$34.75"],
          ["Red Lamp", "$36.20"],
          ["Bookshelf", "$89.99"],
          ["Blue Rug", "$64.10"],
          ["Lamp Shade", "$12.40"],
        ]
        return page("Prices", `<h1>Store prices</h1><table border="1" cellpadding="6"><tr><th>Item</th><th>Price</th></tr>${rows.map(([item, price]) => `<tr><td>${item}</td><td>${price}</td></tr>`).join("")}</table>`)
      }
      if (url.pathname === "/form")
        return page(
          "Sign up",
          `<h1>Sign up</h1><form method="post" action="/submit">
            <p><label>Full name <input name="name"></label></p>
            <p><label>Email <input name="email" type="email"></label></p>
            <p><label>Plan <select name="plan"><option value="">Choose…</option><option value="free">Free</option><option value="pro">Pro</option><option value="team">Team</option></select></label></p>
            <p><label><input type="checkbox" name="terms"> I accept the terms</label></p>
            <p><button type="submit">Create account</button></p></form>`,
        )
      if (url.pathname === "/submit" && request.method === "POST") {
        received.push(Object.fromEntries(new URLSearchParams(await request.text())))
        return page("Thanks", "<h1>Account created</h1><p>Thanks for signing up.</p>")
      }
      if (url.pathname === "/files")
        return page("Files", `<h1>Files</h1><p><a href="/report.csv" download="cryzo-eval-report.csv">Download report</a></p><p><a href="/prices">Other page</a></p>`)
      if (url.pathname === "/report.csv")
        return new Response("item,amount\nlamps,2\nrugs,1\n", {
          headers: { "content-type": "text/csv", "content-disposition": 'attachment; filename="cryzo-eval-report.csv"' },
        })
      if (url.pathname === "/code") return page("Confirmation", "<h1>Order placed</h1><p>Your confirmation code is <strong>QX-7341</strong>.</p>")
      return new Response("not found", { status: 404 })
    },
  })
  return { server, url: `http://127.0.0.1:${server.port}`, received }
}

/** Pass rates count only runs the model service completed; provider failures are listed apart. */
function summarize(results: Result[]) {
  const counted = results.filter((result) => !result.provider)
  const byTask = Map.groupBy(results, (result) => result.task)
  const average = (values: number[]) => values.reduce((sum, value) => sum + value, 0) / Math.max(values.length, 1)
  const rows = [...byTask].map(([task, runs]) => {
    const scored = runs.filter((run) => !run.provider)
    const passed = scored.filter((run) => run.ok)
    const outage = runs.length - scored.length
    return `| ${task} | ${passed.length}/${scored.length}${outage ? ` (+${outage} provider errors)` : ""} | ${average(passed.map((run) => run.seconds)).toFixed(1)}s | ${average(passed.map((run) => run.calls)).toFixed(1)} | ${average(scored.map((run) => run.errors)).toFixed(1)} |`
  })
  const passed = counted.filter((result) => result.ok)
  const outages = results.length - counted.length
  return [
    `**${model.providerID}:${model.id}**: ${passed.length}/${counted.length} passed (${Math.round((100 * passed.length) / Math.max(counted.length, 1))}%), average ${average(passed.map((result) => result.seconds)).toFixed(1)}s per passed task${outages ? `; ${outages} run(s) lost to model provider errors` : ""}`,
    "",
    "| Task | Passed | Avg time (passed) | Avg calls (passed) | Avg tool errors |",
    "|---|---|---|---|---|",
    ...rows,
  ].join("\n")
}

function pickModel(wanted: string | undefined) {
  if (!wanted) {
    const fallback = modelList.find((model) => model.enabled !== false)
    if (!fallback) throw new Error("the engine offers no models; run with --list-models")
    return fallback
  }
  const [providerID, ...rest] = wanted.split(":")
  const id = rest.join(":")
  const found = modelList.find((model) => model.providerID === providerID && model.id === id)
  if (!found) throw new Error(`model ${wanted} is not offered; run with --list-models`)
  return found
}

async function firstLine(stream: ReadableStream<Uint8Array>) {
  const reader = stream.getReader()
  const decoder = new TextDecoder()
  let text = ""
  while (!text.includes("\n")) {
    const chunk = await reader.read()
    if (chunk.done) throw new Error("the engine exited before it started")
    text += decoder.decode(chunk.value)
  }
  reader.releaseLock()
  return JSON.parse(text.split("\n")[0]) as { url: string; token: string }
}

async function stop() {
  engine.kill()
  await engine.exited
  // The engine's data folder holds only eval sessions; keep the results, drop the rest.
  await rm(path.join(work, "data"), { recursive: true, force: true }).catch(() => undefined)
}

function parseArgs(list: string[]) {
  const parsed: Record<string, string | undefined> = {}
  list.forEach((item, index) => {
    if (!item.startsWith("--")) return
    const next = list[index + 1]
    parsed[item.slice(2)] = next && !next.startsWith("--") ? next : "true"
  })
  return parsed as { model?: string; tasks?: string; runs?: string; timeout?: string; engine?: string; "list-models"?: string }
}
