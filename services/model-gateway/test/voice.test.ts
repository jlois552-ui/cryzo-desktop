import { describe, expect, test } from "bun:test"
import { convexTest } from "convex-test"
import schema from "../convex/schema"
import { internal } from "../convex/_generated/api"
import { DAILY_SECONDS, MAX_SECONDS, recording } from "../convex/lib/voice"

const modules = {
  "../convex/voice/store.ts": () => import("../convex/voice/store"),
  "../convex/_generated/server.js": () => import("../convex/_generated/server"),
}

// base64 of a WAV this many seconds long (16 kHz, 16-bit mono, plus the 44-byte header).
const wav = (seconds: number) => Buffer.alloc(44 + seconds * 32_000).toString("base64")
// base64 of a compressed recording (AAC or Opus) of this many bytes.
const compressed = (bytes: number) => Buffer.alloc(bytes).toString("base64")

describe("dictation", () => {
  test("a recording's length comes from its size and long ones are refused", () => {
    expect(recording(wav(5))).toEqual({ seconds: 5 })
    expect(recording(wav(MAX_SECONDS))).toEqual({ seconds: MAX_SECONDS })
    expect("problem" in recording(wav(MAX_SECONDS + 5))).toBe(true)
    expect("problem" in recording("")).toBe(true)
    expect("problem" in recording("not base64!")).toBe(true)
  })

  test("a compressed recording counts its stated length, but never less than its size allows", () => {
    expect(recording(compressed(20_000), 8)).toEqual({ seconds: 8 })
    expect(recording(compressed(20_000))).toEqual({ seconds: 1 })
    expect(recording(compressed(20_000), "8")).toEqual({ seconds: 1 })
    expect(recording(wav(30), 2)).toEqual({ seconds: 30 })
    expect("problem" in recording(compressed(20_000), MAX_SECONDS + 5)).toBe(true)
  })

  test("each user gets a daily allowance", async () => {
    const t = convexTest(schema, modules)
    const day = "2026-11-18"
    await t.mutation(internal.voice.store.spend, { user: "a", day, seconds: DAILY_SECONDS - 10 })
    await expect(t.mutation(internal.voice.store.spend, { user: "a", day, seconds: 20 })).rejects.toThrow()
    await t.mutation(internal.voice.store.spend, { user: "b", day, seconds: 20 })
    await t.mutation(internal.voice.store.spend, { user: "a", day: "2026-11-19", seconds: 20 })
  })
})
