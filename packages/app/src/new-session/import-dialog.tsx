import { createResource, For, Show } from "solid-js"
import { createStore } from "solid-js/store"
import { Button } from "@opencode/ui/button"
import { Checkbox } from "@opencode/ui/checkbox"
import { Dialog, DialogTitle } from "@opencode/ui/dialog"
import { Loader } from "@opencode/ui/loader"
import { useLanguage } from "@/runtime/i18n/language"
import { usePlatform, type Platform } from "@/runtime/platform/platform"
import { useServerSDK } from "@/runtime/server/client"
import { authTokenFromCredentials } from "@/runtime/server/api"

export type ImportableConversation = {
  source: "claude-code" | "codex"
  file: string
  title: string
  directory: string
  exists: boolean
  imported: boolean
  updated: number
}

/** Calls the engine's import endpoint: GET lists the conversations, POST imports the chosen files. */
export async function importRequest(input: {
  url: string
  password?: string
  platform: Platform
  files?: string[]
  fallback: string
}) {
  const response = await (input.platform.fetch ?? fetch)(`${input.url}/api/session/import`, {
    method: input.files ? "POST" : "GET",
    headers: {
      ...(input.password ? { Authorization: `Basic ${authTokenFromCredentials({ password: input.password })}` } : {}),
      ...(input.files ? { "Content-Type": "application/json" } : {}),
    },
    body: input.files ? JSON.stringify({ files: input.files }) : undefined,
  })
  const data = (await response.json().catch(() => ({}))) as Record<string, unknown>
  if (!response.ok) throw new Error(typeof data.message === "string" ? data.message : input.fallback)
  return data
}

/** The conversations still to import: not imported yet, and their folder is still on this computer. */
export function importable(data: Record<string, unknown>) {
  const list = Array.isArray(data.conversations) ? (data.conversations as ImportableConversation[]) : []
  return list.filter((item) => !item.imported && item.exists)
}

/**
 * Import conversations from Claude Code and Codex on this computer, as Cursor and the ChatGPT app
 * do. Each becomes a session in the folder it was held in, with its original dates.
 */
export function DialogImportConversations() {
  const sdk = useServerSDK()
  const platform = usePlatform()
  const language = useLanguage()
  const request = (files?: string[]) =>
    importRequest({
      url: sdk.url,
      password: sdk.http.password,
      platform,
      files,
      fallback: language.t("import.error"),
    })

  const [state, setState] = createStore({
    picked: [] as string[],
    busy: false,
    error: "",
    imported: 0,
    done: false,
  })

  const [conversations] = createResource(() =>
    request().then((data) => {
      const list = importable(data)
      setState("picked", list.map((item) => item.file))
      return list
    }),
  )

  // Reading the resource directly would suspend and blank the dialog while it loads.
  const loaded = () => conversations.state === "ready" || conversations.state === "refreshing"
  const list = () => (loaded() ? conversations.latest : undefined) ?? []
  const groups = () =>
    (["claude-code", "codex"] as const)
      .map((source) => ({ source, items: list().filter((item) => item.source === source) }))
      .filter((group) => group.items.length > 0)

  const run = async () => {
    setState({ busy: true, error: "" })
    await request(state.picked)
      .then((data) => setState({ imported: Array.isArray(data.sessions) ? data.sessions.length : 0, done: true }))
      .catch((error: unknown) => setState("error", error instanceof Error ? error.message : String(error)))
    setState("busy", false)
  }

  return (
    <Dialog fit containerClass="!w-[min(calc(100vw_-_32px),520px)] !rounded-xl">
      <div
        class="flex max-h-[calc(100dvh_-_48px)] min-h-0 w-full flex-col gap-4 overflow-y-auto px-6 pb-6 pt-6 [font-family:var(--v2-font-family-sans)]"
        data-scrollable
      >
        <div class="text-[17px] font-[530] text-v2-text-text-base">
          <DialogTitle>{language.t("import.title")}</DialogTitle>
        </div>
        <p class="text-[13px] leading-5 text-v2-text-text-muted">{language.t("import.description")}</p>

        <Show when={loaded()} fallback={<Loader />}>
          <Show
            when={!state.done}
            fallback={
              <p class="text-[13px] leading-5 text-v2-text-text-base" role="status">
                {language.plural("import.done", state.imported)}
              </p>
            }
          >
            <Show
              when={groups().length > 0}
              fallback={<p class="text-[13px] leading-5 text-v2-text-text-muted">{language.t("import.empty")}</p>}
            >
              <For each={groups()}>
                {(group) => (
                  <div class="flex flex-col gap-2">
                    <p class="text-[12px] font-[530] text-v2-text-text-muted">
                      {language.t(group.source === "codex" ? "import.source.codex" : "import.source.claude")}
                    </p>
                    <For each={group.items}>
                      {(item) => (
                        <Checkbox
                          checked={state.picked.includes(item.file)}
                          description={item.directory}
                          onChange={(checked: boolean) =>
                            setState("picked", (picked) =>
                              checked ? [...picked, item.file] : picked.filter((file) => file !== item.file),
                            )
                          }
                        >
                          {item.title}
                        </Checkbox>
                      )}
                    </For>
                  </div>
                )}
              </For>
              <Button
                variant="contrast"
                size="large"
                disabled={state.busy || state.picked.length === 0}
                onClick={() => void run()}
              >
                {state.busy ? language.t("import.busy") : language.plural("import.action", state.picked.length)}
              </Button>
            </Show>
          </Show>
        </Show>

        <Show when={state.error}>
          <p class="text-[13px] text-v2-state-fg-danger" role="alert">
            {state.error}
          </p>
        </Show>
      </div>
    </Dialog>
  )
}
