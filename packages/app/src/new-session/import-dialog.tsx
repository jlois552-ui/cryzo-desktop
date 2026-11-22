import { createResource, For, Show } from "solid-js"
import { createStore } from "solid-js/store"
import { useNavigate } from "@solidjs/router"
import { Button } from "@opencode/ui/button"
import { Checkbox } from "@opencode/ui/checkbox"
import { useDialog } from "@opencode/ui/context/dialog"
import { Dialog, DialogTitle } from "@opencode/ui/dialog"
import { Loader } from "@opencode/ui/loader"
import { useLanguage } from "@/runtime/i18n/language"
import { usePlatform, type Platform } from "@/runtime/platform/platform"
import { useServerSDK } from "@/runtime/server/client"
import { useServer } from "@/runtime/server/current"
import { useGlobal } from "@/runtime/server/runtime"
import { authTokenFromCredentials } from "@/runtime/server/api"
import { sessionHref } from "@/shell/routes/session"

/** An imported conversation's Cryzo session. `stale` means its transcript grew after the import. */
type ImportedSession = { id: string; directory: string; title: string; file?: string; stale?: boolean }

export type ImportableConversation = {
  source: "claude-code" | "codex"
  file: string
  title: string
  directory: string
  exists: boolean
  imported: boolean
  session?: ImportedSession
  stale?: boolean
  updated: number
}

/** Calls the engine's import endpoint: GET lists the conversations, POST imports the chosen files. */
export async function importRequest(input: {
  url: string
  password?: string
  platform: Platform
  files?: string[]
  /** Import files already imported again, from their newer transcript. */
  update?: boolean
  fallback: string
}) {
  const response = await (input.platform.fetch ?? fetch)(`${input.url}/api/session/import`, {
    method: input.files ? "POST" : "GET",
    headers: {
      ...(input.password ? { Authorization: `Basic ${authTokenFromCredentials({ password: input.password })}` } : {}),
      ...(input.files ? { "Content-Type": "application/json" } : {}),
    },
    body: input.files ? JSON.stringify({ files: input.files, update: input.update }) : undefined,
  })
  const data = (await response.json().catch(() => ({}))) as Record<string, unknown>
  if (!response.ok) throw new Error(typeof data.message === "string" ? data.message : input.fallback)
  return data
}

function conversations(data: Record<string, unknown>) {
  return Array.isArray(data.conversations) ? (data.conversations as ImportableConversation[]) : []
}

/** The conversations still to import: not imported yet, and their folder is still on this computer. */
export function importable(data: Record<string, unknown>) {
  return conversations(data).filter((item) => !item.imported && item.exists)
}

/**
 * Import conversations from Claude Code and Codex on this computer, as Cursor and the ChatGPT app
 * do. Each becomes a session in the folder it was held in, with its original dates, and that folder
 * joins the user's projects so the session shows under it.
 */
export function DialogImportConversations() {
  const sdk = useServerSDK()
  const server = useServer()
  const global = useGlobal()
  const navigate = useNavigate()
  const dialog = useDialog()
  const platform = usePlatform()
  const language = useLanguage()
  const request = (files?: string[], update?: boolean) =>
    importRequest({
      url: sdk.url,
      password: sdk.http.password,
      platform,
      files,
      update,
      fallback: language.t("import.error"),
    })

  const [state, setState] = createStore({
    picked: [] as string[],
    busy: false,
    error: "",
    done: [] as ImportedSession[],
    finished: false,
  })

  const [found] = createResource(() =>
    request().then((data) => {
      const list = importable(data)
      setState("picked", list.map((item) => item.file))
      return {
        todo: list,
        imported: conversations(data).flatMap((item) =>
          item.session ? [{ ...item.session, file: item.file, stale: item.stale }] : [],
        ),
      }
    }),
  )

  // Reading the resource directly would suspend and blank the dialog while it loads.
  const loaded = () => found.state === "ready" || found.state === "refreshing"
  const todo = () => (loaded() ? found.latest?.todo : undefined) ?? []
  const earlier = () => (loaded() ? found.latest?.imported : undefined) ?? []
  const groups = () =>
    (["claude-code", "codex"] as const)
      .map((source) => ({ source, items: todo().filter((item) => item.source === source) }))
      .filter((group) => group.items.length > 0)

  // A session shows in the app under its project, so its folder becomes one first.
  const addProjects = (sessions: ImportedSession[]) =>
    Promise.all(
      [...new Set(sessions.map((session) => session.directory))].map((directory) =>
        global.ensureServerCtx(server.conn).projects.add(directory),
      ),
    )

  const run = async () => {
    setState({ busy: true, error: "" })
    await request(state.picked)
      .then(async (data) => {
        const sessions = Array.isArray(data.sessions) ? (data.sessions as ImportedSession[]) : []
        await addProjects(sessions)
        setState({ done: sessions, finished: true })
      })
      .catch((error: unknown) => setState("error", error instanceof Error ? error.message : String(error)))
    setState("busy", false)
  }

  // Brings in the newer transcript, then opens it in place of the old copy.
  const update = async (session: ImportedSession) => {
    if (!session.file) return
    setState({ busy: true, error: "" })
    const data = await request([session.file], true).catch((error: unknown) => {
      setState("error", error instanceof Error ? error.message : String(error))
      return undefined
    })
    setState("busy", false)
    const fresh = Array.isArray(data?.sessions) ? (data.sessions as ImportedSession[])[0] : undefined
    if (fresh) await open(fresh)
  }

  const open = async (session: ImportedSession) => {
    await addProjects([session])
    navigate(sessionHref(server.key, session.id))
    dialog.close()
  }

  const SessionList = (props: { sessions: ImportedSession[] }) => (
    <div class="flex flex-col gap-1">
      <For each={props.sessions}>
        {(session) => (
          <div class="flex min-w-0 items-center gap-2">
            <div class="flex min-w-0 flex-1 flex-col">
              <span class="truncate text-[13px] leading-[var(--line-height-compact)] text-v2-text-text-base">
                {session.title}
              </span>
              <span class="truncate text-[12px] leading-4 text-v2-text-text-muted">{session.directory}</span>
            </div>
            <Show when={session.stale}>
              <Button size="small" variant="neutral" disabled={state.busy} onClick={() => void update(session)}>
                {state.busy ? language.t("import.busy") : language.t("import.session.update")}
              </Button>
            </Show>
            <Button size="small" variant="ghost-muted" onClick={() => void open(session)}>
              {language.t("import.session.open")}
            </Button>
          </div>
        )}
      </For>
    </div>
  )

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
            when={!state.finished}
            fallback={
              <div class="flex flex-col gap-3" role="status">
                <p class="text-[13px] leading-5 text-v2-text-text-base">
                  {language.plural("import.done", state.done.length)}
                </p>
                <SessionList sessions={state.done} />
              </div>
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
            <Show when={earlier().length > 0}>
              <div class="flex flex-col gap-2 border-t border-v2-border-border-base pt-3">
                <p class="text-[12px] font-[530] text-v2-text-text-muted">{language.t("import.earlier")}</p>
                <SessionList sessions={earlier()} />
              </div>
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
