import type { NoteContextItem } from "@/composer/schema"

type Rect = { x: number; y: number; width: number; height: number }

type Element = {
  tag: string
  id?: string
  classes?: string[]
  attributes?: Record<string, string>
  selector?: string
  text?: string
  rect?: Rect
}

/**
 * Composer notes from the browser pane picker's raw notes (Cursor's Design Mode sends the same details).
 * The page wrote these, so anything malformed is dropped and page strings are quoted to read as data.
 */
export function browserNotes(raw: readonly unknown[], id: () => string): NoteContextItem[] {
  return raw.flatMap((value) => {
    if (!isRecord(value)) return []
    const url = text(value.url, 2000)
    const comment = text(value.comment, 4000) ?? ""
    if (!url) return []
    if (value.kind === "element" && isRecord(value.element)) {
      const element = readElement(value.element)
      if (!element) return []
      return [note(id(), url, comment, label(element), "select-element", elementSubject(element, url))]
    }
    if (value.kind === "area" && isRecord(value.area)) {
      const rect = readRect(value.area.rect)
      if (!rect) return []
      const inside = (Array.isArray(value.area.elements) ? value.area.elements : [])
        .filter(isRecord)
        .slice(0, 10)
        .map((item) => readElement(item))
        .filter((item): item is Element => item !== undefined)
      const parts = inside.map((item) => label(item) + (item.selector ? ` (selector ${JSON.stringify(item.selector)})` : ""))
      return [
        note(
          id(),
          url,
          comment,
          `Area ${rect.width}×${rect.height}`,
          "outline-globe",
          `an area of the page at ${JSON.stringify(url)} at x ${rect.x}, y ${rect.y}, ${rect.width}×${rect.height} pixels` +
            (parts.length ? `, containing ${parts.join("; ")}` : ""),
        ),
      ]
    }
    return []
  })
}

function note(commentID: string, url: string, comment: string, label: string, icon: string, subject: string): NoteContextItem {
  return { type: "note", origin: "browser", commentID, label, icon, subject, comment, href: url }
}

function label(element: Element) {
  const words = element.text ? element.text.slice(0, 40) : (element.attributes?.["aria-label"] ?? element.id ?? "")
  return words ? `${element.tag} ${JSON.stringify(words)}` : element.tag
}

function elementSubject(element: Element, url: string) {
  const details = [
    `tag ${element.tag}`,
    element.id ? `id ${JSON.stringify(element.id)}` : undefined,
    element.classes?.length ? `classes ${JSON.stringify(element.classes.join(" "))}` : undefined,
    element.selector ? `selector ${JSON.stringify(element.selector)}` : undefined,
    element.text ? `text ${JSON.stringify(element.text)}` : undefined,
    element.attributes && Object.keys(element.attributes).length
      ? `attributes ${JSON.stringify(element.attributes)}`
      : undefined,
    element.rect
      ? `at x ${element.rect.x}, y ${element.rect.y}, ${element.rect.width}×${element.rect.height} pixels`
      : undefined,
  ].filter((detail) => detail !== undefined)
  return `the ${JSON.stringify(label(element))} element on the page at ${JSON.stringify(url)} (${details.join("; ")})`
}

function readElement(value: Record<string, unknown>): Element | undefined {
  const tag = text(value.tag, 40)
  if (!tag || !/^[a-z][a-z0-9-]*$/.test(tag)) return
  const attributes = isRecord(value.attributes)
    ? Object.fromEntries(
        Object.entries(value.attributes)
          .slice(0, 20)
          .flatMap(([name, item]) => (typeof item === "string" ? [[name.slice(0, 40), item.slice(0, 200)]] : [])),
      )
    : undefined
  return {
    tag,
    id: text(value.id, 200),
    classes: Array.isArray(value.classes)
      ? value.classes.filter((item): item is string => typeof item === "string").slice(0, 20).map((item) => item.slice(0, 80))
      : undefined,
    attributes,
    selector: text(value.selector, 1000),
    text: text(value.text, 200),
    rect: readRect(value.rect),
  }
}

function readRect(value: unknown): Rect | undefined {
  if (!isRecord(value)) return
  const numbers = [value.x, value.y, value.width, value.height]
  if (!numbers.every((item) => typeof item === "number" && Number.isFinite(item))) return
  const [x, y, width, height] = numbers.map((item) => Math.round(item as number))
  return { x: x!, y: y!, width: width!, height: height! }
}

function text(value: unknown, max: number) {
  if (typeof value !== "string") return
  const trimmed = value.trim()
  return trimmed ? trimmed.slice(0, max) : undefined
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}
