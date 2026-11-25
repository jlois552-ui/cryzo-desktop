import { mkdtemp, writeFile } from "node:fs/promises"
import { createHash } from "node:crypto"
import { createReadStream } from "node:fs"
import { tmpdir } from "node:os"
import path from "node:path"
import { parseArgs } from "node:util"
import { cryzo, vercel, type Backend } from "./faceoff-backends"
import { copyTicket } from "./faceoff-workflow"

const { values } = parseArgs({
  options: {
    "cryzo-mcp": { type: "string" },
    "baseline-cryzo-mcp": { type: "string" },
    "outcome-waits": { type: "boolean" },
    "agent-desktop": { type: "string" },
    runs: { type: "string", default: "5" },
    workflow: { type: "boolean" },
    "target-foreground": { type: "boolean" },
    "isolated-windows": { type: "boolean" },
    "model-bridge": { type: "string" },
    "oauth-db": { type: "string" },
  },
})
const runs = Number(values.runs)
if (process.platform !== "win32") throw new Error("This matrix requires an interactive Windows desktop")
if (!Number.isInteger(runs) || runs < 1 || runs > 30) throw new Error("--runs must be from 1 to 30")
const live = !!values["model-bridge"]
if (live && values.workflow) throw new Error("Choose the scripted workflow or the native model matrix")
if (values["target-foreground"] && !values.workflow) throw new Error("--target-foreground requires --workflow")
if (values["baseline-cryzo-mcp"] && !values.workflow) throw new Error("--baseline-cryzo-mcp requires --workflow")
if (values["outcome-waits"] && !values.workflow) throw new Error("--outcome-waits requires --workflow")
if (live && (!values["isolated-windows"] || !values["oauth-db"])) {
  throw new Error("The model matrix requires --isolated-windows and --oauth-db from the dedicated Windows profile")
}
const root = path.resolve(import.meta.dir, "..", "..")
const cryzoExe = path.resolve(values["cryzo-mcp"] ?? path.join(root, "target", "debug", "cryzo-desktop-mcp.exe"))
const baselineExe = values["baseline-cryzo-mcp"] && path.resolve(values["baseline-cryzo-mcp"])
const vercelExe = path.resolve(values["agent-desktop"] ?? path.join(tmpdir(), "cryzo-agent-desktop-v1.3.1", "agent-desktop.exe"))
for (const exe of [cryzoExe, vercelExe, baselineExe]) {
  if (!exe) continue
  if (!(await Bun.file(exe).exists())) throw new Error("Executable missing: " + exe)
}
const version = Bun.spawnSync([vercelExe, "--version"]).stdout.toString().trim()
if (!/^agent-desktop 1\.3\.1$/m.test(version)) throw new Error("Expected the pinned agent-desktop 1.3.1")
const work = await mkdtemp(path.join(tmpdir(), "cryzo-faceoff-"))
async function digest(file: string) {
  const hash = createHash("sha256")
  for await (const chunk of createReadStream(file)) hash.update(chunk)
  return hash.digest("hex")
}
await writeFile(path.join(work, "builds.json"), JSON.stringify({
  platform: process.platform, architecture: process.arch, runs,
  cryzo: await digest(cryzoExe), vercel: await digest(vercelExe), vercelVersion: version.split("\n")[0],
  ...(baselineExe && { baselineCryzo: await digest(baselineExe) }),
}, null, 2))
await writeFile(path.join(work, "cryzo.json"), JSON.stringify({
  enabled: true,
  approval: "never",
  auditLog: path.join(work, "cryzo-audit.log"),
}))
const factories = {
  cryzo: () => cryzo(cryzoExe, work),
  ...(baselineExe && { "cryzo-baseline": () => cryzo(baselineExe, work) }),
  vercel: () => vercel(vercelExe, work, false, !!values.workflow),
  "vercel-headed": () => vercel(vercelExe, work, true, !!values.workflow),
}
type Variant = keyof typeof factories
type Observation = { foreground: string; target: string; decoy: string; source?: string; focusChanges: number; pointerChanges: number }
type Scripted = {
  backend: Variant
  run: number
  ok: boolean
  fixtureReady: boolean
  elapsed: Partial<Record<"observe" | "locate" | "act", number>>
  targetClicks: number
  wrongWindowClicks: number
  background: boolean
  focusChanged: boolean
  pointerMoved: boolean
  workflow?: Awaited<ReturnType<typeof copyTicket>>
  error?: string
}

async function observation(directory: string, timeout = 2000, stopped = () => false): Promise<Observation> {
  const deadline = performance.now() + timeout
  while (performance.now() < deadline && !stopped()) {
    const value = await Bun.file(path.join(directory, "observation.json")).json().catch(() => null)
    if (value?.target && value?.decoy) return value
    await Bun.sleep(50)
  }
  throw new Error("Fixture did not report its window handles")
}

async function fixtureRun(variant: Variant, run: number): Promise<Scripted> {
  const directory = await mkdtemp(path.join(work, "fixture-"))
  const tag = path.basename(directory).slice(-10)
  const fixture = Bun.spawn([
    "powershell.exe", "-NoProfile", "-Sta", "-WindowStyle", "Hidden", "-File",
    path.join(import.meta.dir, "fixtures", "faceoff.ps1"), "-Directory", directory, "-Tag", tag,
    ...(values.workflow ? ["-Workflow"] : []),
    ...(values["target-foreground"] ? ["-TargetForeground"] : []),
  ], { stdout: "ignore", stderr: "ignore" })
  const backend: Backend = factories[variant]!()
  const elapsed: Scripted["elapsed"] = {}
  async function timed<T>(phase: keyof Scripted["elapsed"], work: () => Promise<T>) {
    const started = performance.now()
    try { return await work() }
    finally { elapsed[phase] = Math.round(performance.now() - started) }
  }
  const outcome: Scripted = {
    backend: variant, run, elapsed, ok: false, fixtureReady: false, targetClicks: 0, wrongWindowClicks: 0,
    background: false, focusChanged: false, pointerMoved: false,
  }
  try {
    const initial = await observation(directory, 15_000, () => fixture.exitCode !== null)
    outcome.fixtureReady = true
    const windows = await backend.windows()
    function match(handle: string, label: string) {
      const id = backend.name === "cryzo" ? "w" + BigInt(handle).toString(16) : "w-" + handle
      return windows.find((window) => window.id === id && window.title === "Cryzo Faceoff " + tag + " " + label)
    }
    const target = match(initial.target, "Target")
    if (!target) throw new Error("Backend could not identify the exact fixture window")
    if (values.workflow) {
      const source = initial.source && match(initial.source, "Source")
      if (!source) throw new Error("Backend could not identify the exact source window")
      const before = await observation(directory)
      if (values["target-foreground"] && before.foreground !== before.target) {
        outcome.fixtureReady = false
        throw new Error("Fixture could not establish a foreground target")
      }
      outcome.background = before.foreground !== before.target
      outcome.workflow = await copyTicket(backend, source, target, directory, tag, !!values["outcome-waits"] && variant === "cryzo")
      const after = await observation(directory)
      outcome.focusChanged = before.focusChanges !== after.focusChanges
      outcome.pointerMoved = before.pointerChanges !== after.pointerChanges
      outcome.ok = outcome.workflow.ok
      outcome.error = outcome.workflow.error
      return outcome
    }
    const opened = await timed("observe", () => backend.inspect(target))
    if (!opened.some((element) => element.name === "Increment")) throw new Error("Snapshot omitted the fixture button")
    const located = await timed("locate", () => backend.inspect(target, "Increment"))
    const [button, ...duplicates] = located.filter((element) => element.role === "button" && element.name === "Increment" && element.ref)
    if (!button || duplicates.length) throw new Error("Locate did not return one exact fixture button")
    const before = await observation(directory)
    outcome.background = before.foreground !== before.target
    try {
      await timed("act", () => backend.act(target, "click", button.ref))
    } finally {
      await Bun.sleep(150)
      const after = await observation(directory)
      outcome.focusChanged = before.focusChanges !== after.focusChanges
      outcome.pointerMoved = before.pointerChanges !== after.pointerChanges
      outcome.targetClicks = Number(await Bun.file(path.join(directory, "target.txt")).text())
      outcome.wrongWindowClicks = Number(await Bun.file(path.join(directory, "decoy.txt")).text())
    }
    outcome.ok = outcome.targetClicks === 1 && outcome.wrongWindowClicks === 0
    if (!outcome.ok) outcome.error = "The fixture scored the delivered click as incorrect"
  } catch (error) {
    outcome.error = String(error)
  } finally {
    await backend.stop()
    fixture.kill()
    await fixture.exited
  }
  return outcome
}

function timing(values: number[], fraction: number) {
  const sorted = values.toSorted((left, right) => left - right)
  if (!sorted.length) return "—"
  if (fraction === .5) {
    const middle = Math.floor(sorted.length / 2)
    return (sorted[middle] + sorted[Math.floor((sorted.length - 1) / 2)]) / 2 + " ms"
  }
  return sorted[Math.ceil(sorted.length * fraction) - 1] + " ms"
}

function scriptedReport(results: Scripted[]) {
  const header = [
    "# Windows desktop-control face-off",
    "",
    "Scripted duplicate-window fixture. Times include the public tool boundary, including a process launch per Vercel CLI call.",
    "",
    "| Backend | Correct | Observe p50/p95 | Locate p50/p95 | Act p50/p95 | Background clicks without sampled focus or pointer change | Wrong-window clicks | Fixture startup errors |",
    "|---|---:|---:|---:|---:|---:|---:|---:|",
  ]
  return header.concat([...Map.groupBy(results, (row) => row.backend)].map(([name, rows]) => {
    const passed = rows.filter((row) => row.ok)
    const phases = (["observe", "locate", "act"] as const).map((phase) => {
      const values = passed.flatMap((row) => row.elapsed[phase] === undefined ? [] : [row.elapsed[phase]])
      return timing(values, .5) + " / " + timing(values, .95)
    })
    const stayed = passed.filter((row) => row.background && !row.focusChanged && !row.pointerMoved).length
    const available = rows.filter((row) => row.fixtureReady).length
    return ["", name, passed.length + "/" + available, ...phases, String(stayed),
      String(rows.reduce((sum, row) => sum + row.wrongWindowClicks, 0)), String(rows.length - available), ""].join(" | ")
  })).join("\n")
}

function workflowReport(results: Scripted[]) {
  const header = [
    "# Windows ticket-copy workflow", "",
    "Read a ticket in one window, copy its ID and two lines of notes into another, mark it reviewed, and save once.",
    "The fixture checks the saved record and checks the matching decoy form for any changes. Timing starts with the first read and ends after the save call returns and the fixture has recorded the save.",
    "Each backend uses one persistent MCP connection per trial.",
    ...(values["outcome-waits"] ? ["The current Cryzo build waits for exact outcomes. The baseline and Vercel use their default action responses."] : []),
    values["target-foreground"] ? "The destination form starts in the foreground." : "The decoy form starts in the foreground.",
    "",
    "| Backend | Verified / trials | Whole workflow p50 / p95 | Median tool calls on passes | Decoy events | Fixture startup errors |",
    "|---|---:|---:|---:|---:|---:|",
  ]
  return header.concat([...Map.groupBy(results, (row) => row.backend)].map(([name, rows]) => {
    const available = rows.filter((row) => row.fixtureReady).length
    const passed = rows.filter((row) => row.ok && row.workflow).map((row) => row.workflow!)
    const ms = passed.map((row) => row.ms)
    const calls = passed.map((row) => row.steps.length).toSorted((a, b) => a - b)
    const decoys = rows.reduce((sum, row) => sum + (row.workflow?.decoyEvents ?? 0), 0)
    return ["", name, passed.length + "/" + available, timing(ms, .5) + " / " + timing(ms, .95),
      calls.length ? calls[Math.ceil(calls.length / 2) - 1] : "—", decoys, rows.length - available, ""].join(" | ")
  })).join("\n")
}

console.log("Reports: " + work)
const results: Scripted[] = []
const reportName = values.workflow ? "workflow" : "scripted"
const variants = Object.keys(factories) as Variant[]
for (let index = 0; index < runs; index++) {
  for (const variant of [...variants.slice(index % variants.length), ...variants.slice(0, index % variants.length)]) {
    for (let attempt = 0; attempt < 2; attempt++) {
      if (process.env.CRYZO_DESKTOP_PROFILE_SETTLE && (variant === "cryzo" || variant === "cryzo-baseline")) {
        console.log(`Profile ${variant} run=${index + 1}`)
      }
      const outcome = await fixtureRun(variant, index + 1)
      results.push(outcome)
      await Bun.write(path.join(work, reportName + ".json"), JSON.stringify(results, null, 2))
      if (outcome.error) console.log(variant, index + 1, outcome.error)
      if (outcome.fixtureReady) break
    }
  }
}
const report = values.workflow ? workflowReport(results) : scriptedReport(results)
console.log(report)
await Bun.write(path.join(work, reportName + ".md"), report + "\n")
if (live) {
  const { nativeMatrix } = await import("./faceoff-native")
  await nativeMatrix({
    work, runs, oauthDb: path.resolve(values["oauth-db"]!), modelBridge: path.resolve(values["model-bridge"]!),
    cryzo: factories.cryzo, vercel: factories["vercel-headed"],
  })
}
