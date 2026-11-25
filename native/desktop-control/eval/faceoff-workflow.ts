import path from "node:path"
import { type Action, type Backend, type Element, type Window } from "./faceoff-backends"

type Saved = { ticket: string; notes: string; reviewed: boolean }

async function fixtureText(file: Bun.BunFile) {
  try { return (await file.text()).trim() }
  catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") return ""
    throw error
  }
}

export async function copyTicket(backend: Backend, source: Window, target: Window, directory: string, tag: string, outcomeWaits = false) {
  const started = performance.now()
  const steps: { tool: string; ms: number }[] = []
  async function measure<T>(tool: string, call: () => Promise<T>) {
    const before = performance.now()
    try { return await call() }
    finally { steps.push({ tool, ms: Math.round(performance.now() - before) }) }
  }
  function readField(elements: Element[], label: string) {
    const rows = elements.filter((element) => element.name.startsWith(label))
    if (rows.length !== 1) throw new Error("Could not read a unique " + label + " value from the source window")
    return rows[0].name.slice(label.length).trim()
  }
  async function operate(name: string, roles: string[], action: Action, value?: string, expected?: string) {
    const results = await measure("find " + name, () => backend.inspect(target, name))
    const matches = results.filter((element) => element.ref && element.name === name && roles.includes(element.role))
    if (matches.length !== 1) throw new Error("Could not locate " + name + " uniquely")
    await measure(action + " " + name, () => backend.act(target, action, matches[0].ref, value, outcomeWaits ? expected : undefined))
  }
  let error: string | undefined
  const file = Bun.file(path.join(directory, "target.saves"))
  try {
    const input = await measure("read ticket", () => backend.inspect(source))
    const ticket = readField(input, "Ticket:")
    const summary = readField(input, "Summary:")
    const location = readField(input, "Location:")
    const editRoles = ["edit", "textfield", "textarea", "document"]
    const notes = summary + "\n" + location
    await operate("Ticket ID", editRoles, "set_text", ticket, 'edit "Ticket ID" value=' + JSON.stringify(ticket))
    await operate("Notes", editRoles, "set_text", notes.replaceAll("\n", "\r\n"), 'edit "Notes" value=' + JSON.stringify(notes))
    await operate("Reviewed", ["checkbox"], "check", undefined, 'checkbox "Reviewed" checked')
    await operate("Save draft", ["button"], "click", undefined, "Draft saved")
    // Confirm the fixture has durably received the save. Include this in elapsed time.
    for (let attempts = 0; attempts < 15 && !(await file.exists()); attempts++) await Bun.sleep(40)
  } catch (failure) {
    error = String(failure)
  }
  const ms = Math.round(performance.now() - started)
  const raw = await fixtureText(file)
  const records: Saved[] = raw ? raw.split(/\r?\n/).map((line) => JSON.parse(line)) : []
  const decoy = await fixtureText(Bun.file(path.join(directory, "decoy.events")))
  const decoyEvents = decoy ? decoy.split(/\r?\n/).length : 0
  const expectedNotes = "Replace scanner label\nRack " + tag
  const saved = records[0]
  const ok = !error && records.length === 1 && !decoyEvents && saved?.ticket === "CUA-" + tag
    && typeof saved.notes === "string" && saved.notes.replaceAll("\r\n", "\n") === expectedNotes && saved.reviewed === true
  if (!ok && !error) {
    error = "Fixture saved " + records.length + " draft(s). Saved record " + JSON.stringify(saved)
      + ". Decoy events " + decoyEvents + "."
  }
  return { ok, ms, steps, saves: records.length, decoyEvents, saved, error }
}
