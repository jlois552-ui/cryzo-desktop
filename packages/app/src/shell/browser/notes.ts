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
    const region = isRegion(value.kind) ? regions[value.kind] : undefined
    const area = region && value[region.field]
    if (!region || !isRecord(area)) return []
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
        `${region.label} ${rect.width}×${rect.height}`,
        region.icon,
        `${region.subject} the page at ${JSON.stringify(url)} at x ${rect.x}, y ${rect.y}, ${rect.width}×${rect.height} pixels` +
          (parts.length ? `, over ${parts.join("; ")}` : ""),
      ),
    ]
  })
}

const regions = {
  area: { field: "area", label: "Area", icon: "outline-globe", subject: "an area of" },
  drawing: { field: "drawing", label: "Drawing", icon: "edit", subject: "a mark the user drew on" },
} as const

function isRegion(kind: unknown): kind is keyof typeof regions {
  return typeof kind === "string" && Object.hasOwn(regions, kind)
}

/**
 * Where on the page the notes point, padded for context and kept within the visible page, in the
 * page's CSS pixels; the screenshot sent with them is cropped to it.
 */
export function noteArea(raw: readonly unknown[]) {
  const records = raw.filter(isRecord)
  const viewport = readRect({ x: 0, y: 0, ...(isRecord(records[0]?.viewport) ? records[0].viewport : {}) })
  const rects = records.flatMap((value) => {
    const holder = [value.element, value.area, value.drawing].find(isRecord)
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
