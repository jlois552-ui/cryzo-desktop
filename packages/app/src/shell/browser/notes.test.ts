import { describe, expect, test } from "bun:test"
import { browserNotes, browserSelection, noteArea } from "./notes"

let next = 0
const id = () => `c${++next}`

describe("browser pane notes", () => {
  test("an element pick carries what Cursor's Design Mode sends, quoted as data", () => {
    const [note] = browserNotes(
      [
        {
          kind: "element",
          url: "http://localhost:5173/",
          comment: "Make this bigger",
          element: {
            tag: "button",
            id: "save",
            classes: ["btn", "primary"],
            attributes: { type: "submit" },
            selector: "#save",
            text: 'Save "now"',
            rect: { x: 10, y: 20, width: 80, height: 32 },
          },
        },
      ],
      id,
    )
    expect(note).toMatchObject({
      type: "note",
      origin: "browser",
      label: 'button "Save \\"now\\""',
      icon: "select-element",
      comment: "Make this bigger",
      href: "http://localhost:5173/",
    })
    expect(note!.subject).toContain('selector "#save"')
    expect(note!.subject).toContain('attributes {"type":"submit"}')
    expect(note!.subject).toContain("at x 10, y 20, 80×32 pixels")
  })

  test("an area pick lists the elements inside it", () => {
    const [note] = browserNotes(
      [
        {
          kind: "area",
          url: "http://localhost:5173/",
          comment: "",
          area: { rect: { x: 0, y: 0, width: 300, height: 120 }, elements: [{ tag: "h1", selector: "h1", text: "Hello" }] },
        },
      ],
      id,
    )
    expect(note!.label).toBe("Area 300×120")
    expect(note!.subject).toContain('h1 "Hello" (selector "h1")')
  })

  test("an edited element carries the previewed changes and the component that rendered it", () => {
    const [note] = browserNotes(
      [
        {
          kind: "element",
          url: "http://localhost:5173/",
          comment: "Make it red",
          element: { tag: "button", component: { framework: "react", names: ["SaveButton", "Toolbar"] } },
          changes: [
            { property: "color", from: "rgb(0, 0, 0)", to: "rgb(225, 29, 72)" },
            { property: "text", from: "Save", to: "Save changes" },
            { property: "position", from: "static", to: "fixed" },
            { property: "padding-top", from: 1, to: "12px" },
          ],
        },
      ],
      id,
    )
    expect(note!.subject).toContain("rendered by react component SaveButton in Toolbar")
    expect(note!.subject).toContain(
      'requested changes, previewed by the user: color "rgb(0, 0, 0)" → "rgb(225, 29, 72)", text "Save" → "Save changes"',
    )
    expect(note!.subject).not.toContain("position")
    expect(note!.subject).not.toContain("padding-top")
  })

  test("the inspector's selection keeps only the styles it shows", () => {
    expect(
      browserSelection({
        element: { tag: "h1", selector: "h1" },
        styles: { color: "rgb(0, 0, 0)", position: "fixed", "font-size": 32 },
        text: "Hello",
      }),
    ).toEqual({
      element: { tag: "h1", selector: "h1", classes: undefined, attributes: undefined, id: undefined, text: undefined, rect: undefined, component: undefined },
      styles: { color: "rgb(0, 0, 0)" },
      text: "Hello",
    })
    expect(browserSelection({ element: { tag: "<img>" } })).toBeUndefined()
    expect(browserSelection("x")).toBeUndefined()
  })

  test("the screenshot area covers every note with room around it, within the visible page", () => {
    const viewport = { width: 800, height: 600 }
    expect(
      noteArea([
        { kind: "element", viewport, element: { tag: "a", rect: { x: 20, y: 30, width: 50, height: 20 } } },
        { kind: "area", viewport, area: { rect: { x: 500, y: 400, width: 200, height: 180 } } },
      ]),
    ).toEqual({ viewport: { x: 0, y: 0, ...viewport }, rect: { x: 0, y: 0, width: 796, height: 600 } })
    expect(noteArea([{ kind: "element", element: { tag: "a", rect: { x: 0, y: 0, width: 5, height: 5 } } }])).toBeUndefined()
  })

  test("malformed picks from the page are dropped", () => {
    expect(
      browserNotes(
        [
          null,
          "x",
          { kind: "element", url: "http://a/", element: { tag: "<script>" } },
          { kind: "element", element: { tag: "div" } },
          { kind: "area", url: "http://a/", area: { rect: { x: "1" } } },
          { kind: "other", url: "http://a/" },
          { kind: "toString", url: "http://a/" },
          { kind: "drawing", url: "http://a/", drawing: { rect: { x: 0, y: 0, width: 9, height: 9 } } },
        ],
        id,
      ),
    ).toEqual([])
  })
})
