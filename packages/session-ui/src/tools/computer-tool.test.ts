import { describe, expect, test } from "bun:test"
import { createUiI18n, pluralCategory, pluralKey, resolveTemplate } from "@opencode/ui/context/i18n"
import { dict as en } from "@opencode/ui/i18n/en"
import { computerStep } from "./computer-step"

// The real English dictionary, so the test reads the lines a user sees.
const i18n = createUiI18n({
  locale: () => "en",
  t: (key, params) => resolveTemplate(en[key as keyof typeof en] ?? String(key), params),
  plural: (key, count, params) =>
    resolveTemplate(en[pluralKey(key, pluralCategory("en", count)) as keyof typeof en] ?? String(key), {
      count,
      ...params,
    }),
})

const desktop = (command: string, args: Record<string, unknown>) => ({ tool: "desktop", command, args })

describe("computerStep", () => {
  test.each([
    ["desktop_snapshot", desktop("snapshot", { window: "w1a2b3c" }), 'window [w1a2b3c] "ChatGPT" (ChatGPT.exe)', "Read ChatGPT"],
    ["desktop_snapshot", { window: "Notepad" }, undefined, "Read Notepad"],
    ["desktop_snapshot", {}, undefined, "Read the screen"],
    ["desktop_act", desktop("act", { id: "e12", action: "click" }), 'click [e12] button "Send": done', "Clicked Send"],
    ["desktop_act", desktop("act", { name: "Text editor", action: "type", value: "my secret" }), undefined, "Typed into Text editor"],
    ["desktop_key", desktop("key", { keys: "{ctrl}s" }), undefined, "Pressed Ctrl+S"],
    ["desktop_key", desktop("key", { keys: "{enter}" }), undefined, "Pressed Enter"],
    ["desktop_key", desktop("key", { keys: "hello there" }), undefined, "Typed text"],
    ["desktop_launch", desktop("launch", { app: "C:\\Windows\\calc.exe" }), undefined, "Opened calc.exe"],
    ["desktop_batch", desktop("batch", { steps: [{}, {}, {}, {}] }), undefined, "Did 4 steps"],
    ["desktop_browser_open", desktop("browser_open", { url: "https://example.com/a?b" }), undefined, "Opened example.com"],
    ["page_read", {}, undefined, "Read the preview"],
    ["preview", { program: "npm", args: ["run", "dev"] }, undefined, "Started the dev server"],
  ] as const)("%s reads as a plain line", (name, input, output, expected) => {
    expect(computerStep(i18n, name, input, output)).toBe(expected)
  })

  test("typed text never shows in the line", () => {
    const line = computerStep(i18n, "desktop_act", desktop("act", { name: "Password", action: "type", value: "hunter2" }), undefined)
    expect(line).not.toContain("hunter2")
  })
})
