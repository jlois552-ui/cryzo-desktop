import { For, type ParentProps, Show } from "solid-js"
import { createStore } from "solid-js/store"
import { Button } from "@opencode/ui/button"
import { useLanguage } from "@/runtime/i18n/language"
import type { BrowserSelection, InspectorStyle } from "./notes"

type Section = { title: string; styles: InspectorStyle[]; grid?: boolean; when?: () => boolean }

/** Common values offered for keyword properties; any other value can still be typed. */
const options: Partial<Record<InspectorStyle, string[]>> = {
  "font-weight": ["300", "400", "500", "600", "700", "800"],
  "text-align": ["left", "center", "right", "justify"],
  display: ["block", "inline-block", "inline", "flex", "inline-flex", "grid", "none"],
  "flex-direction": ["row", "column", "row-reverse", "column-reverse"],
  "align-items": ["stretch", "flex-start", "center", "flex-end", "baseline"],
  "justify-content": ["flex-start", "center", "flex-end", "space-between", "space-around", "space-evenly"],
}

const colors: InspectorStyle[] = ["color", "background-color", "border-top-color"]

/**
 * Cursor's Design Mode and ChatGPT's Adjust, for the element the user chose to edit: its styles as the
 * page computed them, each change previewed live on the page, then sent to the agent as a note.
 */
export function BrowserInspector(props: {
  selection: BrowserSelection
  width: number
  onStyle: (property: string, value: string) => void
  onReset: () => void
  onCancel: () => void
  onSend: (comment: string) => void
}) {
  const language = useLanguage()
  const [state, setState] = createStore({
    values: { ...props.selection.styles } as Partial<Record<InspectorStyle, string>>,
    text: props.selection.text,
    comment: "",
  })
  const flex = () => /flex|grid/.test(state.values.display ?? "")
  const sections = (): Section[] => [
    {
      title: language.t("browser.inspector.typography"),
      styles: ["font-family", "font-size", "font-weight", "line-height", "letter-spacing", "text-align", "color"],
    },
    { title: language.t("browser.inspector.fill"), styles: ["background-color", "opacity"] },
    {
      title: language.t("browser.inspector.margin"),
      styles: ["margin-top", "margin-right", "margin-bottom", "margin-left"],
      grid: true,
    },
    {
      title: language.t("browser.inspector.padding"),
      styles: ["padding-top", "padding-right", "padding-bottom", "padding-left"],
      grid: true,
    },
    { title: language.t("browser.inspector.border"), styles: ["border-radius", "border-top-width", "border-top-color"] },
    { title: language.t("browser.inspector.layout"), styles: ["display", "width", "height"] },
    {
      title: language.t("browser.inspector.flex"),
      styles: ["flex-direction", "gap", "align-items", "justify-content"],
      when: flex,
    },
  ]
  const change = (property: InspectorStyle, value: string) => {
    setState("values", property, value)
    props.onStyle(property, value)
  }
  const element = () => props.selection.element
  const component = () => element().component

  return (
    <section
      aria-label={language.t("browser.inspector.title")}
      class="flex min-h-0 shrink-0 flex-col overflow-hidden rounded-[8px] bg-v2-background-bg-base"
      style={{ width: `${props.width - 6}px` }}
    >
      <header class="flex shrink-0 flex-col gap-0.5 border-b border-v2-border-border-muted px-3 py-2">
        <span class="truncate text-13-medium leading-[var(--line-height-compact)] text-v2-text-text-base">
          {language.t("browser.inspector.title")}
        </span>
        <span class="truncate font-mono text-12-regular text-v2-text-text-muted" dir="ltr">
          {element().tag}
          {element().id ? `#${element().id}` : element().classes?.[0] ? `.${element().classes![0]}` : ""}
          <Show when={element().rect}>{(rect) => ` ${rect().width}×${rect().height}`}</Show>
        </span>
        <Show when={component()}>
          {(item) => (
            <span class="truncate font-mono text-12-regular text-v2-text-text-muted" dir="ltr">
              {[item().names.join(" ‹ "), item().file].filter(Boolean).join(" · ")}
            </span>
          )}
        </Show>
      </header>
      <div data-scrollable class="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-3 py-2">
        <textarea
          class="min-h-[56px] resize-none rounded-[6px] border border-v2-border-border-base bg-v2-background-bg-base px-2 py-1.5 text-13-regular text-v2-text-text-base"
          placeholder={language.t("browser.inspector.comment")}
          aria-label={language.t("browser.inspector.comment")}
          value={state.comment}
          onInput={(event) => setState("comment", event.currentTarget.value)}
        />
        <Show when={state.text !== undefined}>
          <Group title={language.t("browser.inspector.text")}>
            <input
              class={field}
              aria-label={language.t("browser.inspector.text")}
              value={state.text ?? ""}
              onInput={(event) => {
                setState("text", event.currentTarget.value)
                props.onStyle("text", event.currentTarget.value)
              }}
            />
          </Group>
        </Show>
        <For each={sections().filter((section) => !section.when || section.when())}>
          {(section) => (
            <Group title={section.title}>
              <div class={section.grid ? "grid grid-cols-4 gap-1" : "flex flex-col gap-1"}>
                <For each={section.styles}>
                  {(property) => (
                    <label class={section.grid ? "flex min-w-0 flex-col gap-0.5" : "flex min-w-0 items-center gap-2"}>
                      <span
                        class="shrink-0 truncate font-mono text-11-regular text-v2-text-text-muted"
                        classList={{ "w-[96px]": !section.grid }}
                        dir="ltr"
                      >
                        {section.grid ? property.split("-").at(-1) : property}
                      </span>
                      <span class="flex min-w-0 flex-1 items-center gap-1">
                        <Show when={colors.includes(property) && hex(state.values[property] ?? "")}>
                          {(color) => (
                            <input
                              type="color"
                              class="size-6 shrink-0 cursor-pointer rounded-[4px] border border-v2-border-border-base bg-transparent p-0"
                              aria-label={property}
                              value={color()}
                              onInput={(event) => change(property, event.currentTarget.value)}
                            />
                          )}
                        </Show>
                        <input
                          class={field}
                          dir="ltr"
                          spellcheck={false}
                          aria-label={property}
                          list={options[property] ? `browser-inspector-${property}` : undefined}
                          value={state.values[property] ?? ""}
                          onInput={(event) => change(property, event.currentTarget.value)}
                        />
                        <Show when={options[property]}>
                          {(values) => (
                            <datalist id={`browser-inspector-${property}`}>
                              <For each={values()}>{(value) => <option value={value} />}</For>
                            </datalist>
                          )}
                        </Show>
                      </span>
                    </label>
                  )}
                </For>
              </div>
            </Group>
          )}
        </For>
      </div>
      <footer class="flex shrink-0 items-center gap-1.5 border-t border-v2-border-border-muted px-3 py-2">
        <Button
          variant="ghost-muted"
          size="small"
          onClick={() => {
            setState({ values: { ...props.selection.styles }, text: props.selection.text })
            props.onReset()
          }}
        >
          {language.t("browser.inspector.reset")}
        </Button>
        <span class="flex-1" />
        <Button variant="neutral" size="small" onClick={() => props.onCancel()}>
          {language.t("browser.inspector.cancel")}
        </Button>
        <Button variant="contrast" size="small" onClick={() => props.onSend(state.comment)}>
          {language.t("browser.inspector.send")}
        </Button>
      </footer>
    </section>
  )
}

const field =
  "h-6 min-w-0 flex-1 rounded-[6px] border border-v2-border-border-base bg-v2-background-bg-base px-1.5 font-mono text-12-regular text-v2-text-text-base"

function Group(props: ParentProps<{ title: string }>) {
  return (
    <div class="flex flex-col gap-1.5">
      <span class="text-12-medium text-v2-text-text-base">{props.title}</span>
      {props.children}
    </div>
  )
}

/** The color as `#rrggbb` for the color input; undefined when transparent or not an rgb() color. */
function hex(value: string) {
  const match = /^rgba?\(\s*(\d+)[,\s]+(\d+)[,\s]+(\d+)(?:[,\s/]+([\d.]+%?))?\s*\)$/.exec(value.trim())
  if (match) {
    if (match[4] !== undefined && parseFloat(match[4]) === 0) return
    return "#" + match.slice(1, 4).map((part) => Number(part).toString(16).padStart(2, "0")).join("")
  }
  return /^#[\da-f]{6}$/i.test(value.trim()) ? value.trim() : undefined
}
