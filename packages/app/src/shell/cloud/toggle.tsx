import { createEffect, on, Show } from "solid-js"
import { createStore } from "solid-js/store"
import { Button } from "@opencode/ui/button"
import { Icon } from "@opencode/ui/icon"
import { Tooltip } from "@opencode/ui/tooltip"
import { useLanguage } from "@/runtime/i18n/language"
import { usePlatform } from "@/runtime/platform/platform"
import { useServerSDK } from "@/runtime/server/client"
import { useDialog } from "@opencode/ui/context/dialog"
import { browserPanel } from "@/shell/browser/panel"
import { cloudRequest } from "./api"
import { DialogPublish } from "./publish-dialog"

/**
 * Local or Cloud for a project folder (Cursor's environment picker). In the cloud, files stay in
 * the folder and the agent's commands run in a Cryzo Cloud workspace; Preview opens the
 * workspace's dev server in the browser pane.
 */
export function CloudToggle(props: { directory: string }) {
  const sdk = useServerSDK()
  const platform = usePlatform()
  const language = useLanguage()
  const dialog = useDialog()
  const [state, setState] = createStore({ loaded: false, linked: false, busy: false, error: "", port: "5173" })

  const call = (path: "cloud" | "cloud/link" | "cloud/unlink" | "cloud/preview", body?: unknown) =>
    cloudRequest({
      url: sdk.url,
      password: sdk.http.password,
      platform,
      directory: props.directory,
      path,
      body,
      fallback: language.t("cloud.error"),
    })

  const run = async (work: () => Promise<void>) => {
    setState({ busy: true, error: "" })
    await work().catch((error: unknown) => setState("error", error instanceof Error ? error.message : String(error)))
    setState("busy", false)
  }

  createEffect(
    on(
      () => props.directory,
      () =>
        void call("cloud")
          .then((data) => setState({ loaded: true, linked: data.linked === true }))
          .catch(() => setState({ loaded: true, linked: false })),
    ),
  )

  const toggle = () =>
    run(async () => {
      const data = await call(state.linked ? "cloud/unlink" : "cloud/link")
      setState("linked", data.linked === true)
    })

  const preview = () =>
    run(async () => {
      const data = await call("cloud/preview", { port: Number(state.port) })
      if (typeof data.url === "string") browserPanel.openAt(data.url)
    })

  return (
    <Show when={state.loaded}>
      <div class="flex items-center gap-1.5 ps-2 text-[13px] leading-[var(--line-height-compact)]">
        <Tooltip value={language.t("cloud.toggle.description")} placement="top" openDelay={500}>
          <Button
            size="small"
            variant={state.linked ? "neutral" : "ghost-muted"}
            icon="cloud-upload"
            aria-pressed={state.linked}
            disabled={state.busy}
            data-action="cloud-toggle"
            onClick={() => void toggle()}
          >
            {state.linked ? language.t("cloud.toggle.cloud") : language.t("cloud.toggle.local")}
          </Button>
        </Tooltip>
        <Show when={state.linked && platform.browserPane}>
          <input
            class="h-6 w-16 rounded-[6px] border border-v2-border-border-base bg-v2-background-bg-base px-1.5 font-mono text-12-regular text-v2-text-text-base"
            value={state.port}
            inputmode="numeric"
            aria-label={language.t("cloud.preview.port")}
            onInput={(event) => setState("port", event.currentTarget.value)}
          />
          <Button size="small" variant="ghost-muted" disabled={state.busy} onClick={() => void preview()}>
            <Icon name="square-arrow-top-right" size="small" />
            {language.t("cloud.preview")}
          </Button>
        </Show>
        <Button
          size="small"
          variant="ghost-muted"
          data-action="publish"
          onClick={() => void dialog.show(() => <DialogPublish directory={props.directory} />)}
        >
          <Icon name="globe" size="small" />
          {language.t("publish.button")}
        </Button>
        <Show when={state.error}>
          <span class="text-12-regular text-v2-state-fg-danger" role="alert">
            {state.error}
          </span>
        </Show>
      </div>
    </Show>
  )
}
