import { describe, expect, test } from "bun:test"
import { browserNotes } from "./notes"

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
        ],
        id,
      ),
    ).toEqual([])
  })
})
