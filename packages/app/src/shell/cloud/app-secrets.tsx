import { createResource, For, Show } from "solid-js"
import { createStore } from "solid-js/store"
import { Button } from "@opencode/ui/button"
import { TextField } from "@opencode/ui/text-field"
import { useLanguage } from "@/runtime/i18n/language"

type Call = (
  path: "apps/secrets" | "apps/secrets/set" | "apps/secrets/delete",
  body: unknown,
) => Promise<Record<string, unknown>>

/**
 * A published app's secrets (API keys, OAuth client secrets), as on Cursor and Lovable: the user
 * types a value once, it goes straight to the app's host, and only the name is ever shown again,
 * to the user or the agent.
 */
export function AppSecrets(props: { slug: string; call: Call }) {
  const language = useLanguage()
  const [state, setState] = createStore({ name: "", value: "", busy: false, error: "", confirm: "" })
  const [names, { refetch }] = createResource(
    () => props.slug,
    (slug) =>
      props
        .call("apps/secrets", { slug })
        .then((data) => (Array.isArray(data.secrets) ? (data.secrets as string[]) : [])),
  )

  // Reading the resource directly would suspend and blank the dialog while it loads.
  const list = () => (names.state === "ready" || names.state === "refreshing" ? names.latest : undefined) ?? []

  const run = async (work: () => Promise<unknown>) => {
    setState({ busy: true, error: "" })
    const done = await work().then(
      () => true,
      (error: unknown) => {
        setState("error", error instanceof Error ? error.message : String(error))
        return false
      },
    )
    setState("busy", false)
    if (done) void refetch()
    return done
  }

  const save = async () => {
    const saved = await run(() =>
      props.call("apps/secrets/set", { slug: props.slug, name: state.name, value: state.value }),
    )
    // The value leaves this form either way once it is saved; it is never kept or shown.
    if (saved) setState({ name: "", value: "" })
  }

  return (
    <div class="flex flex-col gap-3 rounded-lg border border-v2-border-border-base p-3">
      <p class="text-[12px] leading-4 text-v2-text-text-muted">{language.t("secrets.description")}</p>
      <Show when={names.error}>
        <p class="text-[12px] text-v2-state-fg-danger" role="alert">
          {names.error instanceof Error ? names.error.message : language.t("cloud.error")}
        </p>
      </Show>
      <Show when={list().length > 0}>
        <ul class="flex flex-col gap-1">
          <For each={list()}>
            {(name) => (
              <li class="flex items-center gap-2 text-[13px]">
                <span class="min-w-0 flex-1 truncate font-mono text-v2-text-text-base">{name}</span>
                <Show
                  when={state.confirm === name}
                  fallback={
                    <Button
                      size="small"
                      variant="ghost-muted"
                      disabled={state.busy}
                      onClick={() => setState("confirm", name)}
                    >
                      {language.t("secrets.remove")}
                    </Button>
                  }
                >
                  <Button size="small" variant="ghost-muted" onClick={() => setState("confirm", "")}>
                    {language.t("common.cancel")}
                  </Button>
                  <Button
                    size="small"
                    variant="neutral"
                    disabled={state.busy}
                    onClick={() =>
                      void run(() => props.call("apps/secrets/delete", { slug: props.slug, name })).then(() =>
                        setState("confirm", ""),
                      )
                    }
                  >
                    {language.t("secrets.remove")}
                  </Button>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Show>
      <form
        class="flex flex-col gap-2"
        onSubmit={(event) => {
          event.preventDefault()
          void save()
        }}
      >
        <TextField
          label={language.t("secrets.name")}
          placeholder="OPENAI_API_KEY"
          value={state.name}
          onChange={(value) => setState("name", value.toUpperCase().replace(/[^A-Z0-9_]/g, "_"))}
          disabled={state.busy}
          autocomplete="off"
          spellcheck={false}
        />
        <TextField
          label={language.t("secrets.value")}
          type="password"
          value={state.value}
          onChange={(value) => setState("value", value)}
          disabled={state.busy}
          autocomplete="new-password"
        />
        <Show when={state.error}>
          <p class="text-[12px] text-v2-state-fg-danger" role="alert">
            {state.error}
          </p>
        </Show>
        <Button type="submit" variant="neutral" size="large" disabled={state.busy || !state.name || !state.value}>
          {state.busy ? language.t("secrets.busy") : language.t("secrets.save")}
        </Button>
      </form>
    </div>
  )
}
