// The evaluation's tasks. Each check reads the real end state (files, the fixture server, Windows
// itself) rather than trusting the model's answer, except where the task is to report something.

import { mkdir, readdir, rm, writeFile } from "node:fs/promises"
import { existsSync } from "node:fs"
import path from "node:path"

export type Context = {
  /** A fresh folder for this run. */
  dir: string
  /** The local fixture web server, like http://127.0.0.1:51234. */
  web: string
  /** Form posts the fixture server received during this run. */
  received: Record<string, string>[]
  downloads: string
  /** Unique per run, for file names: Notepad restores old tabs, and a tab left by an earlier run
   * with the same name was edited instead of this run's file. */
  tag: string
}

type Verdict = { ok: boolean; detail: string }

type Task = {
  id: string
  prompt: (context: Context) => string
  setup?: (context: Context) => Promise<void>
  check: (context: Context, answer: string) => Promise<Verdict>
  cleanup?: (context: Context) => Promise<void>
}

// Said once per task, as a user who wants the job done on screen would.
const onScreen = "Do this on my PC with computer use (the desktop tools), not with shell commands or file tools."

export const tasks: Task[] = [
  {
    id: "calculator",
    prompt: () => `${onScreen} Open the Windows Calculator app, work out 847 × 293 by clicking its buttons, tell me the result, then close Calculator.`,
    check: async (_, answer) => verdict(/248,?171/.test(answer), `answer ${quote(answer)}`),
    cleanup: () => kill("CalculatorApp.exe"),
  },
  {
    id: "notepad-new",
    prompt: (context) =>
      `${onScreen} Open Notepad, type exactly these two lines:\nCryzo eval line one\nSecond line: 42 apples\nThen save the file as ${path.join(context.dir, `notes-${context.tag}.txt`)} and close that Notepad tab.`,
    check: async (context) => {
      const file = path.join(context.dir, `notes-${context.tag}.txt`)
      if (!existsSync(file)) return verdict(false, "notes.txt was not saved")
      const text = normalize(await Bun.file(file).text())
      return verdict(text === "Cryzo eval line one\nSecond line: 42 apples", `file ${quote(text)}`)
    },
  },
  {
    id: "notepad-edit",
    setup: (context) => writeFile(path.join(context.dir, `draft-${context.tag}.txt`), "The meeting is on Monday.\r\nBring the slides.\r\n"),
    prompt: (context) =>
      `${onScreen} Open ${path.join(context.dir, `draft-${context.tag}.txt`)} in Notepad, change "Monday" to "Friday", save it, and close that Notepad tab.`,
    check: async (context) => {
      const text = normalize(await Bun.file(path.join(context.dir, `draft-${context.tag}.txt`)).text())
      return verdict(text === "The meeting is on Friday.\nBring the slides.", `file ${quote(text)}`)
    },
  },
  {
    id: "explorer",
    setup: async (context) => {
      const inbox = path.join(context.dir, "inbox")
      await mkdir(inbox, { recursive: true })
      await Promise.all(["a.txt", "b.txt", "keep.md"].map((name) => writeFile(path.join(inbox, name), name)))
    },
    prompt: (context) =>
      `${onScreen} In File Explorer, open the folder ${path.join(context.dir, "inbox")}, create a new folder in it named Archive, and move a.txt and b.txt into Archive. Leave keep.md where it is.`,
    check: async (context) => {
      const inbox = path.join(context.dir, "inbox")
      const archived = ["a.txt", "b.txt"].every((name) => existsSync(path.join(inbox, "Archive", name)))
      const left = await readdir(inbox)
      return verdict(archived && !left.includes("a.txt") && !left.includes("b.txt") && left.includes("keep.md"), `inbox now has ${left.join(", ")}`)
    },
    cleanup: (context) => closeExplorer(path.join(context.dir, "inbox")),
  },
  {
    id: "settings",
    prompt: () => `${onScreen} Using the Windows Settings app, find this PC's Windows edition and its full OS build number, tell me both, then close Settings.`,
    check: async (_, answer) => {
      const build = await powershell(
        "$v = Get-ItemProperty 'HKLM:\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion'; \"$($v.CurrentBuild).$($v.UBR)\"",
      )
      const edition = (await powershell("(Get-CimInstance Win32_OperatingSystem).Caption")).replace(/^Microsoft Windows \d+ /, "")
      return verdict(answer.includes(build) && answer.toLowerCase().includes(edition.toLowerCase()), `expected ${edition} ${build}; answer ${quote(answer)}`)
    },
    cleanup: () => kill("SystemSettings.exe"),
  },
  {
    id: "browser-table",
    prompt: (context) => `${onScreen} In the Cryzo browser, open ${context.web}/prices and tell me the price of the Blue Lamp.`,
    check: async (_, answer) => verdict(answer.includes("34.75"), `answer ${quote(answer)}`),
  },
  {
    id: "browser-form",
    prompt: (context) =>
      `${onScreen} In the Cryzo browser, open ${context.web}/form and sign up with the name Ada Lovelace, the email ada@example.com and the Pro plan, accept the terms, and submit the form.`,
    check: async (context) => {
      const form = context.received.at(-1)
      const ok = form?.name === "Ada Lovelace" && form?.email === "ada@example.com" && form?.plan === "pro" && form?.terms === "on"
      return verdict(ok, form ? `server got ${JSON.stringify(form)}` : "the form was never submitted")
    },
  },
  {
    id: "browser-download",
    setup: (context) => removeReports(context.downloads),
    prompt: (context) => `${onScreen} In the Cryzo browser, open ${context.web}/files, download the report, and tell me where it was saved.`,
    check: async (context) => {
      const reports = await reportsIn(context.downloads)
      const contents = await Promise.all(reports.map((name) => Bun.file(path.join(context.downloads, name)).text()))
      return verdict(contents.some((text) => normalize(text) === "item,amount\nlamps,2\nrugs,1"), reports.length ? `downloaded ${reports.join(", ")}` : "no report in Downloads")
    },
    cleanup: (context) => removeReports(context.downloads),
  },
  {
    id: "canvas-ocr",
    setup: async () => {
      game = Bun.spawn(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", path.join(import.meta.dir, "fixtures", "game.ps1")], {
        stdout: "ignore",
        stderr: "ignore",
      })
      await Bun.sleep(2500)
    },
    prompt: () => `${onScreen} A window titled "Cryzo Eval Game" is open. What score and what level does it show?`,
    check: async (_, answer) => verdict(answer.includes("4200") && /level\D{0,12}7|7\D{0,6}level/i.test(answer), `answer ${quote(answer)}`),
    cleanup: async () => {
      game?.kill()
      game = undefined
    },
  },
  {
    id: "cross-app",
    prompt: (context) =>
      `${onScreen} In the Cryzo browser, open ${context.web}/code and read the confirmation code. Then open Notepad, put only that code in a new file, save it as ${path.join(context.dir, `code-${context.tag}.txt`)}, and close that Notepad tab.`,
    check: async (context) => {
      const file = path.join(context.dir, `code-${context.tag}.txt`)
      if (!existsSync(file)) return verdict(false, "code.txt was not saved")
      const text = normalize(await Bun.file(file).text())
      return verdict(text === "QX-7341", `file ${quote(text)}`)
    },
  },
]

let game: ReturnType<typeof Bun.spawn> | undefined

function verdict(ok: boolean, detail: string): Verdict {
  return { ok, detail: detail.slice(0, 300) }
}

function normalize(text: string) {
  return text.replace(/^﻿/, "").replace(/\r\n/g, "\n").trim()
}

function quote(text: string) {
  return JSON.stringify(text.replace(/\s+/g, " ").slice(0, 160))
}

async function powershell(command: string) {
  const process = Bun.spawn(["powershell", "-NoProfile", "-Command", command], { stdout: "pipe", stderr: "ignore" })
  return (await new Response(process.stdout).text()).trim()
}

async function kill(image: string) {
  await Bun.spawn(["taskkill", "/im", image, "/f"], { stdout: "ignore", stderr: "ignore" }).exited
}

/** Closes Explorer windows showing `folder`, never Explorer itself, which is the taskbar too. */
async function closeExplorer(folder: string) {
  const target = folder.replace(/'/g, "''")
  await powershell(
    `(New-Object -ComObject Shell.Application).Windows() | Where-Object { $_.Document.Folder.Self.Path -like '${target}*' } | ForEach-Object { $_.Quit() }`,
  )
}

async function reportsIn(downloads: string) {
  return (await readdir(downloads)).filter((name) => /^cryzo-eval-report.*\.csv$/.test(name))
}

async function removeReports(downloads: string) {
  await Promise.all((await reportsIn(downloads)).map((name) => rm(path.join(downloads, name), { force: true })))
}
