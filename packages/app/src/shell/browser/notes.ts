import type { NoteContextItem } from "@/composer/schema"

type Rect = { x: number; y: number; width: number; height: number }

type Component = { framework: string; names: string[]; file?: string }

type Element = {
  tag: string
  id?: string
  classes?: string[]
  attributes?: Record<string, string>
  selector?: string
  text?: string
  rect?: Rect
  component?: Component
}

type Change = { property: string; from: string; to: string }

/** The CSS properties the inspector shows and the picker lets it change, in the order shown. */
export const inspectorStyles = [
  "color",
  "background-color",
  "opacity",
  "font-family",
  "font-size",
  "font-weight",
  "line-height",
  "letter-spacing",
  "text-align",
  "margin-top",
  "margin-right",
  "margin-bottom",
  "margin-left",
  "padding-top",
  "padding-right",
  "padding-bottom",
  "padding-left",
  "border-radius",
  "border-top-width",
  "border-top-color",
  "display",
  "width",
  "height",
  "flex-direction",
  "gap",
  "align-items",
  "justify-content",
] as const

export type InspectorStyle = (typeof inspectorStyles)[number]

/** The element the inspector edits, as the page's picker described it. */
export type BrowserSelection = {
  element: Element
  styles: Partial<Record<InspectorStyle, string>>
  /** Its text, when the element holds only text and the inspector may change it. */
  text?: string
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
      const changes = readChanges(value.changes)
      return [note(id(), url, comment, label(element), "select-element", elementSubject(element, url, changes))]
    }
    const area = value.kind === "area" && isRecord(value.area) ? value.area : undefined
    if (!area) return []
    const rect = readRect(area.rect)
    if (!rect) return []
    const inside = (Array.isArray(area.elements) ? area.elements : [])
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
  })
}

/** The inspector's element from the picker's raw selection; undefined when anything essential is malformed. */
export function browserSelection(raw: unknown): BrowserSelection | undefined {
  if (!isRecord(raw) || !isRecord(raw.element)) return
  const element = readElement(raw.element)
  if (!element) return
  const styles = isRecord(raw.styles) ? raw.styles : {}
  return {
    element,
    styles: Object.fromEntries(
      inspectorStyles.flatMap((name) => {
        const value = styles[name]
        return typeof value === "string" ? [[name, value.slice(0, 200)]] : []
      }),
    ),
    text: typeof raw.text === "string" ? raw.text.slice(0, 500) : undefined,
  }
}

/**
 * Where on the page the notes point, padded for context and kept within the visible page, in the
 * page's CSS pixels; the screenshot sent with them is cropped to it.
 */
export function noteArea(raw: readonly unknown[]) {
  const records = raw.filter(isRecord)
  const viewport = readRect({ x: 0, y: 0, ...(isRecord(records[0]?.viewport) ? records[0].viewport : {}) })
  const rects = records.flatMap((value) => {
    const holder = [value.element, value.area].find(isRecord)
    const rect = holder && readRect(holder.rect)
    return rect ? [rect] : []
  })
  if (!viewport || viewport.width < 1 || viewport.height < 1 || !rects.length) return
  const pad = 96
  const left = Math.max(0, Math.min(...rects.map((rect) => rect.x)) - pad)
  const top = Math.max(0, Math.min(...rects.map((rect) => rect.y)) - pad)
  const right = Math.min(viewport.width, Math.max(...rects.map((rect) => rect.x + rect.width)) + pad)
  const bottom = Math.min(viewport.height, Math.max(...rects.map((rect) => rect.y + rect.height)) + pad)
  if (right - left < 1 || bottom - top < 1) return
  return { viewport, rect: { x: left, y: top, width: right - left, height: bottom - top } }
}

/** The host's base64 PNG of the page, cropped to `area`, as an image the composer can attach. */
export async function annotationImage(screenshot: string, area: NonNullable<ReturnType<typeof noteArea>>) {
  const bytes = Uint8Array.from(atob(screenshot), (char) => char.charCodeAt(0))
  const bitmap = await createImageBitmap(new Blob([bytes], { type: "image/png" }))
  // The screenshot is in device pixels; the notes are in CSS pixels.
  const scale = bitmap.width / area.viewport.width
  const canvas = new OffscreenCanvas(Math.round(area.rect.width * scale), Math.round(area.rect.height * scale))
  const context = canvas.getContext("2d")
  if (!context) return bitmap.close()
  context.drawImage(bitmap, area.rect.x * scale, area.rect.y * scale, canvas.width, canvas.height, 0, 0, canvas.width, canvas.height)
  bitmap.close()
  return new File([await canvas.convertToBlob({ type: "image/png" })], "annotation.png", { type: "image/png" })
}

function note(commentID: string, url: string, comment: string, label: string, icon: string, subject: string): NoteContextItem {
  return { type: "note", origin: "browser", commentID, label, icon, subject, comment, href: url }
}

function label(element: Element) {
  const words = element.text ? element.text.slice(0, 40) : (element.attributes?.["aria-label"] ?? element.id ?? "")
  return words ? `${element.tag} ${JSON.stringify(words)}` : element.tag
}

function elementSubject(element: Element, url: string, changes: Change[]) {
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
    element.component ? `rendered by ${componentText(element.component)}` : undefined,
    changes.length
      ? `requested changes, previewed by the user: ${changes
          .map((change) => `${change.property} ${JSON.stringify(change.from)} → ${JSON.stringify(change.to)}`)
          .join(", ")}`
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
    component: readComponent(value.component),
  }
}

function readComponent(value: unknown): Component | undefined {
  if (!isRecord(value)) return
  const framework = text(value.framework, 20)
  if (!framework || !/^[a-z]+$/.test(framework)) return
  const names = Array.isArray(value.names)
    ? value.names
        .filter((item): item is string => typeof item === "string")
        .slice(0, 3)
        .map((item) => item.slice(0, 80))
    : []
  const file = text(value.file, 500)
  if (!names.length && !file) return
  return { framework, names, file }
}

function componentText(component: Component) {
  const names = component.names.length ? `${component.framework} component ${component.names.join(" in ")}` : component.framework
  return component.file ? `${names} (${JSON.stringify(component.file)})` : names
}

/** The style and text changes the user previewed in the inspector; malformed entries are dropped. */
function readChanges(value: unknown): Change[] {
  if (!Array.isArray(value)) return []
  return value
    .filter(isRecord)
    .slice(0, 40)
    .flatMap((item) => {
      const property = text(item.property, 40)
      if (!property || (property !== "text" && !(inspectorStyles as readonly string[]).includes(property))) return []
      if (typeof item.from !== "string" || typeof item.to !== "string") return []
      return [{ property, from: item.from.slice(0, 500), to: item.to.slice(0, 500) }]
    })
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
