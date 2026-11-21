// The same tasks as run.ts, for an agent Cryzo doesn't drive itself, such as the ChatGPT app's
// computer use. A small local server prepares each task and checks its real end state, so another
// agent can be timed on identical work:
//
//   bun native/desktop-control/eval/external.ts --agent chatgpt
//   GET  /start?task=calculator   prepares the task and returns { prompt }
//   POST /check?task=calculator   with { answer, seconds, steps?, images? } checks it and cleans up
//   GET  /summary                 the results so far, as a Markdown table
//
// The prompts are the same as Cryzo's, minus the Cryzo-specific wording ("the desktop tools", "the
// Cryzo browser"), so the other agent picks its own way to do each task on screen.

import { mkdir, mkdtemp, writeFile } from "node:fs/promises"
import { homedir, tmpdir } from "node:os"
import path from "node:path"
import { startFixtures } from "./fixtures"
import { tasks, type Context } from "./tasks"

const agent = process.argv[process.argv.indexOf("--agent") + 1] ?? "external"
const work = await mkdtemp(path.join(tmpdir(), `cryzo-eval-${agent}-`))
const fixtures = startFixtures()
const contexts = new Map<string, Context>()
const started = new Map<string, number>()
const results: { task: string; run: number; ok: boolean; detail: string; seconds: number; steps?: number; images?: number }[] = []

const server = Bun.serve({
  hostname: "127.0.0.1",
  port: Number(process.env.EVAL_PORT ?? 4799),
  async fetch(request) {
    const url = new URL(request.url)
    const task = tasks.find((item) => item.id === url.searchParams.get("task"))
    if (url.pathname === "/summary") return new Response(summary())
    if (!task) return Response.json({ error: "unknown task" }, { status: 404 })
    if (url.pathname === "/start") {
      const run = results.filter((result) => result.task === task.id).length + 1
      const dir = path.join(work, `${task.id}-${run}`)
      await mkdir(dir, { recursive: true })
      const context: Context = {
        dir,
        web: fixtures.url,
        received: fixtures.received,
        downloads: path.join(homedir(), "Downloads"),
        tag: `${agent.slice(0, 3)}${Date.now().toString(36).slice(-4)}${run}`,
      }
      fixtures.received.length = 0
      await task.setup?.(context)
      contexts.set(task.id, context)
      started.set(task.id, performance.now())
      return Response.json({ run, prompt: neutral(task.prompt(context)) })
    }
    if (url.pathname === "/check" && request.method === "POST") {
      const context = contexts.get(task.id)
      if (!context) return Response.json({ error: "start the task first" }, { status: 400 })
      const body = (await request.json()) as { answer?: string; seconds?: number; steps?: number; images?: number }
      const verdict = await task.check(context, body.answer ?? "").catch((error: Error) => ({ ok: false, detail: `check failed: ${error.message}` }))
      await task.cleanup?.(context)
      const result = {
        task: task.id,
        run: results.filter((item) => item.task === task.id).length + 1,
        ok: verdict.ok,
        detail: verdict.detail,
        seconds: body.seconds ?? (performance.now() - (started.get(task.id) ?? performance.now())) / 1000,
        steps: body.steps,
        images: body.images,
      }
      results.push(result)
      await writeFile(path.join(work, "results.json"), JSON.stringify({ agent, results }, null, 2))
      return Response.json(result)
    }
    return Response.json({ error: "not found" }, { status: 404 })
  },
})

console.log(JSON.stringify({ url: `http://127.0.0.1:${server.port}`, work }))

/** The task in words any computer-use agent understands. */
function neutral(prompt: string) {
  return prompt
    .replace("with computer use (the desktop tools)", "with computer use")
    .replaceAll("In the Cryzo browser", "In a web browser")
}

function summary() {
  const average = (values: number[]) => values.reduce((sum, value) => sum + value, 0) / Math.max(values.length, 1)
  const rows = Map.groupBy(results, (result) => result.task)
  const passed = results.filter((result) => result.ok)
  return [
    `**${agent}**: ${passed.length}/${results.length} passed, average ${average(passed.map((result) => result.seconds)).toFixed(1)}s per passed task`,
    "",
    "| Task | Passed | Avg time (passed) | Details |",
    "|---|---|---|---|",
    ...[...rows].map(
      ([task, runs]) =>
        `| ${task} | ${runs.filter((run) => run.ok).length}/${runs.length} | ${average(runs.filter((run) => run.ok).map((run) => run.seconds)).toFixed(1)}s | ${runs.map((run) => run.detail.replace(/\|/g, "/")).join("; ")} |`,
    ),
  ].join("\n")
}
