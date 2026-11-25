import { expect, test } from "bun:test"
import { mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import path from "node:path"
import { type Backend } from "./faceoff-backends"
import { copyTicket } from "./faceoff-workflow"

for (const scenario of ["exact", "wrong notes", "duplicate save", "decoy touched"] as const) {
  test("ticket workflow grader: " + scenario, async () => {
    const directory = await mkdtemp(path.join(tmpdir(), "ticket-workflow-test-"))
    const source = { id: "source", title: "Source", app: "fixture", focused: false, area: 100 }
    const target = { ...source, id: "target", title: "Target" }
    const draft = { ticket: "", notes: "", reviewed: false }
    const targetRoles: Record<string, string> = { "Ticket ID": "edit", Notes: "textfield", Reviewed: "checkbox", "Save draft": "button" }
    const backend: Backend = {
      name: "fixture",
      async windows() { return [source, target] },
      async inspect(window, name) {
        return window.id === source.id
          ? ["Ticket: CUA-demo", "Summary: Replace scanner label", "Location: Rack demo"].map((name) => ({ name, role: "text" }))
          : name ? [{ name, role: targetRoles[name], ref: name }] : []
      },
      async act(window, action, ref, value) {
        expect(window.id).toBe(target.id)
        if (ref === "Ticket ID") draft.ticket = value!
        else if (ref === "Notes") draft.notes = scenario === "wrong notes" ? "Dropped content" : value!
        else if (action === "check") draft.reviewed = true
        else {
          const save = JSON.stringify(draft) + "\n"
          await Bun.write(path.join(directory, "target.saves"), scenario === "duplicate save" ? save + save : save)
          if (scenario === "decoy touched") await Bun.write(path.join(directory, "decoy.events"), "text ticket\n")
        }
      },
      async stop() {},
    }
    try {
      const result = await copyTicket(backend, source, target, directory, "demo")
      expect(result.ok).toBe(scenario === "exact")
      expect(result.steps).toHaveLength(9)
    } finally {
      await rm(directory, { recursive: true, force: true })
    }
  })
}
