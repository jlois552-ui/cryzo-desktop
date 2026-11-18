import { describe, expect, test } from "bun:test"
import { inside, parseHashes, previewHost, workspaceRoute } from "../src/route"
import { verify } from "../src/ticket"
import { sign } from "../../model-gateway/convex/lib/ticket"

describe("workspace routes", () => {
  test("only known actions on valid workspace names route", () => {
    expect(workspaceRoute("/w/cryzo-abc123/exec")).toEqual({ workspace: "cryzo-abc123", action: "exec" })
    expect(workspaceRoute("/w/cryzo-abc123/fs/read")).toEqual({ workspace: "cryzo-abc123", action: "read" })
    expect(workspaceRoute("/w/cryzo-abc123/pty")).toEqual({ workspace: "cryzo-abc123", action: "pty" })
    expect(workspaceRoute("/w/cryzo-abc123/shutdown")).toBeNull()
    expect(workspaceRoute("/w/Bad_Name/exec")).toBeNull()
    expect(workspaceRoute("/w/-x/exec")).toBeNull()
  })

  test("previews live on their own hostname under the app domain", () => {
    expect(previewHost("5173-cryzo-abc123.preview.cryzo.dev", "cryzo.dev")).toEqual({ port: 5173, workspace: "cryzo-abc123" })
    expect(previewHost("5173-cryzo-abc123.cryzo.dev", "cryzo.dev")).toBeNull()
    expect(previewHost("22-cryzo-abc123.preview.cryzo.dev", "cryzo.dev")).toBeNull()
    expect(previewHost("5173-cryzo-abc123.preview.evil.app", "cryzo.dev")).toBeNull()
  })

  test("file hashes are read from sha256sum output", () => {
    const hash = "a".repeat(64)
    expect(parseHashes(`${hash}  ./src/app.tsx\n${hash} *./my file.txt\nnoise\n`)).toEqual({ "src/app.tsx": hash, "my file.txt": hash })
  })

  test("paths stay inside the project", () => {
    expect(inside("src/app.tsx")).toBe("/workspace/src/app.tsx")
    expect(inside("/workspace/src/../package.json")).toBe("/workspace/package.json")
    expect(inside(".")).toBe("/workspace")
    for (const escape of ["../etc/passwd", "/etc/passwd", "src/../../x", "/workspaces/x", "a\0b"]) expect(inside(escape)).toBeNull()
  })
})

describe("tickets", () => {
  test("the Worker accepts workspace tickets the gateway signs with the shared key", async () => {
    const key = "k".repeat(48)
    const exp = Date.now() + 60_000
    const ticket = await sign({ role: "workspace", host: "cryzo-abc123", exp }, key)
    expect(await verify(ticket, key)).toEqual({ role: "workspace", host: "cryzo-abc123", exp })
    expect(await verify(ticket, "z".repeat(48))).toBeNull()
  })
})
