import { createResource, For, Show } from "solid-js"
import { createStore } from "solid-js/store"
import { Button } from "@opencode/ui/button"
import { Dialog, DialogTitle } from "@opencode/ui/dialog"
import { TextField } from "@opencode/ui/text-field"
import { useLanguage } from "@/runtime/i18n/language"
import { usePlatform } from "@/runtime/platform/platform"
import { useServerSDK } from "@/runtime/server/client"
import { cloudRequest } from "./api"
import { AppSecrets } from "./app-secrets"

type Problem = { path: string; reason: string }
type App = { slug: string; url: string | null }

/**
 * Publish, like Cursor's tray and Lovable's Publish button: pick the address, run the checks,
 * and get the live link. Problems are listed so the user or the agent can fix them first, and a
 * publish that would delete live data waits for the user's yes.
 */
export function DialogPublish(props: { directory: string }) {
  const sdk = useServerSDK()
  const platform = usePlatform()
  const language = useLanguage()
  const folder = props.directory.split(/[\\/]/).filter(Boolean).at(-1) ?? "app"
  const [state, setState] = createStore({
    slug:
      folder
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/^-+|-+$/g, "")
        .slice(0, 40) || "my-app",
    busy: false,
    publishing: false,
    error: "",
    url: "",
    warning: "",
    problems: [] as Problem[],
    risky: [] as Problem[],
    manage: "",
    confirm: "",
    downloaded: "",
  })

  const call = (
    path:
      | "publish"
      | "apps"
      | "apps/unpublish"
      | "apps/data"
      | "apps/secrets"
      | "apps/secrets/set"
      | "apps/secrets/delete",
    body?: unknown,
  ) =>
    cloudRequest({
      url: sdk.url,
      password: sdk.http.password,
      platform,
      directory: props.directory,
      path,
      body,
      fallback: language.t("cloud.error"),
    })

  const [apps, { refetch }] = createResource(() =>
    call("apps")
      .then((data) => (Array.isArray(data.apps) ? (data.apps as App[]) : []))
      .catch(() => [] as App[]),
  )

  // Reading the resource directly would suspend and blank the dialog while it loads.
  const published = () => (apps.state === "ready" || apps.state === "refreshing" ? apps.latest : undefined) ?? []

  const run = async (work: () => Promise<void>) => {
    setState({ busy: true, error: "" })
    await work().catch((error: unknown) => setState("error", error instanceof Error ? error.message : String(error)))
    setState("busy", false)
  }

  const publish = (confirm = false) =>
    run(async () => {
      setState({ url: "", warning: "", problems: [], risky: [], publishing: true })
      const data = await call("publish", { slug: state.slug, confirm }).finally(() => setState("publishing", false))
      setState({
        problems: Array.isArray(data.problems) ? (data.problems as Problem[]) : [],
        risky: Array.isArray(data.confirm) ? (data.confirm as Problem[]) : [],
        url: typeof data.url === "string" ? data.url : "",
        warning: typeof data.warning === "string" ? data.warning : "",
      })
      void refetch()
    })

  const unpublish = (slug: string) =>
    run(async () => {
      await call("apps/unpublish", { slug })
      setState({ confirm: "", manage: "", url: "" })
      void refetch()
    })

  const download = (slug: string) =>
    run(async () => {
      setState("downloaded", "")
      const data = await call("apps/data", { slug })
      setState("downloaded", typeof data.path === "string" ? data.path : "")
    })

  return (
    <Dialog fit containerClass="!w-[min(calc(100vw_-_32px),440px)] !rounded-xl">
      <div
        class="flex max-h-[calc(100dvh_-_48px)] min-h-0 w-full flex-col gap-4 overflow-y-auto px-6 pb-6 pt-6 [font-family:var(--v2-font-family-sans)]"
        data-scrollable
      >
        <div class="text-[17px] font-[530] text-v2-text-text-base">
          <DialogTitle>{language.t("publish.title")}</DialogTitle>
        </div>
        <p class="text-[13px] leading-5 text-v2-text-text-muted">{language.t("publish.description")}</p>
        <form
          class="flex flex-col gap-3"
          onSubmit={(event) => {
            event.preventDefault()
            void publish()
          }}
        >
          <TextField
            label={language.t("publish.address")}
            description={language.t("publish.address.description")}
            value={state.slug}
            onChange={(value) => setState("slug", value.toLowerCase())}
            disabled={state.busy}
            autofocus
          />
          <Button type="submit" variant="contrast" size="large" disabled={state.busy || !state.slug}>
            {state.publishing ? language.t("publish.busy") : language.t("publish.action")}
          </Button>
        </form>

        <Show when={state.error}>
          <p class="text-[13px] text-v2-state-fg-danger" role="alert">
            {state.error}
          </p>
        </Show>

        <Show when={state.problems.length > 0}>
          <div class="flex flex-col gap-1.5" role="alert">
            <p class="text-[13px] font-[530] text-v2-state-fg-danger">{language.t("publish.problems")}</p>
            <ProblemList problems={state.problems} />
          </div>
        </Show>

        <Show when={state.risky.length > 0}>
          <div class="flex flex-col gap-2" role="alert">
            <p class="text-[13px] font-[530] text-v2-state-fg-danger">{language.t("publish.risky")}</p>
            <ProblemList problems={state.risky} />
            <div class="flex gap-2">
              <Button variant="ghost-muted" size="large" onClick={() => setState("risky", [])}>
                {language.t("common.cancel")}
              </Button>
              <Button variant="neutral" size="large" disabled={state.busy} onClick={() => void publish(true)}>
                {language.t("publish.risky.confirm")}
              </Button>
            </div>
          </div>
        </Show>

        <Show when={state.url}>
          <div class="flex flex-col gap-2">
            <p class="text-[13px] font-[530] text-v2-text-text-base">{language.t("publish.live")}</p>
            <Show when={state.warning}>
              <p class="text-[12px] leading-4 text-v2-text-text-muted">{state.warning}</p>
            </Show>
            <TextField
              value={state.url}
              readOnly
              copyable
              copyKind="link"
              label={language.t("publish.link")}
              hideLabel
            />
            <Button variant="neutral" size="large" onClick={() => platform.openExternal(state.url)}>
              {language.t("publish.open")}
            </Button>
          </div>
        </Show>

        <Show when={published().length > 0}>
          <div class="flex flex-col gap-1.5 border-t border-v2-border-border-base pt-3">
            <p class="text-[12px] font-[530] text-v2-text-text-muted">{language.t("publish.apps")}</p>
            <For each={published()}>
              {(app) => (
                <div class="flex flex-col gap-2">
                  <div class="flex items-center gap-2 text-[13px]">
                    <span class="min-w-0 flex-1 truncate font-mono text-v2-text-text-base">{app.url ?? app.slug}</span>
                    <Show when={app.url}>
                      {(url) => (
                        <Button size="small" variant="ghost-muted" onClick={() => platform.openExternal(url())}>
                          {language.t("publish.open")}
                        </Button>
                      )}
                    </Show>
                    <Button
                      size="small"
                      variant={state.manage === app.slug ? "neutral" : "ghost-muted"}
                      aria-expanded={state.manage === app.slug}
                      onClick={() =>
                        setState({ manage: state.manage === app.slug ? "" : app.slug, confirm: "", downloaded: "" })
                      }
                    >
                      {language.t("publish.manage")}
                    </Button>
                  </div>
                  <Show when={state.manage === app.slug}>
                    <div class="flex flex-col gap-3">
                      <Show when={app.url}>
                        <AppSecrets slug={app.slug} call={call} />
                        <div class="flex flex-col gap-1.5 rounded-lg border border-v2-border-border-base p-3">
                          <p class="text-[12px] leading-4 text-v2-text-text-muted">
                            {language.t("publish.data.description")}
                          </p>
                          <Button
                            variant="neutral"
                            size="large"
                            disabled={state.busy}
                            onClick={() => void download(app.slug)}
                          >
                            {language.t("publish.data")}
                          </Button>
                          <Show when={state.downloaded}>
                            <p class="break-all text-[12px] leading-4 text-v2-text-text-base">
                              {language.t("publish.data.saved", { path: state.downloaded })}
                            </p>
                          </Show>
                        </div>
                      </Show>
                      <Show
                        when={state.confirm === app.slug}
                        fallback={
                          <Button variant="ghost-muted" size="large" onClick={() => setState("confirm", app.slug)}>
                            {language.t("publish.unpublish")}
                          </Button>
                        }
                      >
                        <div class="flex items-center gap-2">
                          <span class="flex-1 text-[12px] text-v2-state-fg-danger">
                            {language.t("publish.unpublish.confirm")}
                          </span>
                          <Button size="small" variant="ghost-muted" onClick={() => setState("confirm", "")}>
                            {language.t("common.cancel")}
                          </Button>
                          <Button
                            size="small"
                            variant="neutral"
                            disabled={state.busy}
                            onClick={() => void unpublish(app.slug)}
                          >
                            {language.t("publish.unpublish")}
                          </Button>
                        </div>
                      </Show>
                    </div>
                  </Show>
                </div>
              )}
            </For>
          </div>
        </Show>
      </div>
    </Dialog>
  )
}

function ProblemList(props: { problems: Problem[] }) {
  return (
    <ul class="flex max-h-40 flex-col gap-1 overflow-auto text-[12px] leading-4" data-scrollable>
      <For each={props.problems}>
        {(problem) => (
          <li class="text-v2-text-text-muted">
            <span class="font-mono text-v2-text-text-base">{problem.path}</span> {problem.reason}
          </li>
        )}
      </For>
    </ul>
  )
}
