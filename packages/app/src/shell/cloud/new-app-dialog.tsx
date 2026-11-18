import { Show } from "solid-js"
import { createStore } from "solid-js/store"
import { Button } from "@opencode/ui/button"
import { useDialog } from "@opencode/ui/context/dialog"
import { Dialog, DialogTitle } from "@opencode/ui/dialog"
import { TextField } from "@opencode/ui/text-field"
import { useLanguage } from "@/runtime/i18n/language"
import { usePlatform } from "@/runtime/platform/platform"
import { useServerSDK } from "@/runtime/server/client"
import { useServer } from "@/runtime/server/current"
import { useGlobal } from "@/runtime/server/runtime"
import { useTabs } from "@/shell/tabs/tabs"
import { cloudRequest } from "./api"

/**
 * ChatGPT Sites' "@Sites", as `/app`: name the app and say what it should do. Cryzo creates the
 * project from its starter (site, API, database and sign-in) and opens a session there with the
 * request ready to send.
 */
export function DialogNewApp(props: { directory: string }) {
  const sdk = useServerSDK()
  const server = useServer()
  const global = useGlobal()
  const tabs = useTabs()
  const dialog = useDialog()
  const platform = usePlatform()
  const language = useLanguage()
  const [state, setState] = createStore({ name: "", idea: "", busy: false, error: "" })

  const create = async () => {
    setState({ busy: true, error: "" })
    const created = await cloudRequest({
      url: sdk.url,
      password: sdk.http.password,
      platform,
      directory: props.directory,
      path: "apps/new",
      body: { name: state.name.trim() },
      fallback: language.t("cloud.error"),
    }).catch((error: unknown) => {
      setState({ busy: false, error: error instanceof Error ? error.message : String(error) })
      return undefined
    })
    if (typeof created?.directory !== "string") return
    await global.ensureServerCtx(server.conn).projects.add(created.directory)
    await tabs.newDraft({ server: server.key, directory: created.directory }, state.idea.trim())
    dialog.close()
  }

  return (
    <Dialog fit containerClass="!w-[min(calc(100vw_-_32px),440px)] !rounded-xl">
      <form
        class="flex w-full flex-col gap-4 px-6 pb-6 pt-6 [font-family:var(--v2-font-family-sans)]"
        onSubmit={(event) => {
          event.preventDefault()
          void create()
        }}
      >
        <div class="text-[17px] font-[530] text-v2-text-text-base">
          <DialogTitle>{language.t("newApp.title")}</DialogTitle>
        </div>
        <p class="text-[13px] leading-5 text-v2-text-text-muted">{language.t("newApp.description")}</p>
        <TextField
          label={language.t("newApp.name")}
          value={state.name}
          placeholder="My app"
          onChange={(value) => setState("name", value)}
          disabled={state.busy}
          autofocus
        />
        <TextField
          label={language.t("newApp.idea")}
          value={state.idea}
          placeholder={language.t("newApp.idea.placeholder")}
          onChange={(value) => setState("idea", value)}
          disabled={state.busy}
          multiline
        />
        <Show when={state.error}>
          <p class="text-[13px] text-v2-state-fg-danger" role="alert">
            {state.error}
          </p>
        </Show>
        <Button type="submit" variant="contrast" size="large" disabled={state.busy}>
          {state.busy ? language.t("newApp.busy") : language.t("newApp.action")}
        </Button>
      </form>
    </Dialog>
  )
}
