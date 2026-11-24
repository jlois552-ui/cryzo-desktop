import { For, Show, createResource } from "solid-js"
import type { JSX } from "solid-js"
import { createStore } from "solid-js/store"
import { Schema } from "effect"
import { Button } from "@opencode/ui/button"
import { Loader } from "@opencode/ui/loader"
import { TextField } from "@opencode/ui/text-field"
import { useLanguage } from "@/runtime/i18n/language"
import { usePlatform } from "@/runtime/platform/platform"
import { authTokenFromCredentials } from "@/runtime/server/api"
import { useServerSDK } from "@/runtime/server/client"
import { SettingsList } from "@/settings/list"

const Catalog = Schema.Struct({
  entries: Schema.Array(
    Schema.Struct({
      id: Schema.String,
      name: Schema.String,
      publisher: Schema.String,
      description: Schema.String,
      endpoint: Schema.String,
      tool: Schema.optional(Schema.String),
    }),
  ),
  installed: Schema.Array(Schema.String),
})

const decodeRegistry = Schema.decodeUnknownSync(Schema.Struct({ entries: Schema.Array(Schema.Struct({
  name: Schema.String, description: Schema.String, transports: Schema.Array(Schema.String), packages: Schema.Array(Schema.String),
})) }))

const decodeCatalog = Schema.decodeUnknownSync(Catalog)

export function McpMarketplace(props: { fallback: JSX.Element }) {
  const sdk = useServerSDK()
  const platform = usePlatform()
  const language = useLanguage()
  const [state, setState] = createStore({ browse: false, query: "", pending: "", error: false })

  const [lookup, setLookup] = createStore({ query: "", attempt: 0 })
  const headersForServer = () => {
    const headers = new Headers()
    if (sdk.http.password) headers.set("Authorization", `Basic ${authTokenFromCredentials({ password: sdk.http.password })}`)
    return headers
  }
  const [registry] = createResource(
    () => state.browse && lookup.query === state.query.trim() && !!lookup.query && { query: lookup.query, attempt: lookup.attempt },
    async ({ query }) => {
      try {
        const response = await (platform.fetch ?? fetch)(`${sdk.url}/api/cryzo/mcp-marketplace/registry?search=${encodeURIComponent(query)}`, { headers: headersForServer() })
        if (!response.ok) throw new Error()
        return decodeRegistry(await response.json()).entries
      } catch { return null }
    },
  )

  const request = async (change?: { id: string; installed: boolean }) => {
    const headers = headersForServer()
    if (change) headers.set("Content-Type", "application/json")

    const response = await (platform.fetch ?? fetch)(`${sdk.url}/api/cryzo/mcp-marketplace`, {
      method: change ? "POST" : "GET",
      headers,
      body: change ? JSON.stringify(change) : undefined,
    })

    if (!change && response.status === 404) return null
    if (!response.ok) throw new Error(language.t("common.requestFailed"))

    const catalog = decodeCatalog(await response.json())
    return {
      entries: catalog.entries.map((entry) => ({ ...entry, host: new URL(entry.endpoint).hostname })),
      installed: catalog.installed,
    }
  }

  const [catalog, { mutate, refetch }] = createResource(
    () => sdk.connection.status() === "connected",
    () => request().catch(() => undefined),
  )

  const loaded = () => catalog.state === "ready" || catalog.state === "refreshing"
  const current = () => (loaded() ? catalog.latest : undefined)
  const installed = (id: string) => current()?.installed.includes(id) ?? false

  const visible = () => {
    const query = state.query.trim().toLowerCase()

    return (current()?.entries ?? []).filter((entry) =>
      state.browse
        ? `${entry.name} ${entry.publisher} ${entry.description} ${entry.host}`.toLowerCase().includes(query)
        : installed(entry.id),
    )
  }

  const change = async (id: string) => {
    setState({ pending: id, error: false })
    await request({ id, installed: !installed(id) })
      .then(mutate)
      .catch(() => setState("error", true))
    setState("pending", "")
  }

  return (
    <>
      <Show when={!state.browse || !current()}>{props.fallback}</Show>
      <Show when={catalog.loading || (loaded() && current() !== null)}>
        <div class="settings-section" aria-busy={catalog.loading}>
          <div class="flex flex-wrap items-center justify-between gap-2">
            <h3 class="settings-extension-heading text-13-medium">{language.t("mcp.marketplace.title")}</h3>
            <Show when={current()}>
              <Button size="small" variant="neutral" onClick={() => setState({ browse: !state.browse, query: "" })}>
                {language.t(state.browse ? "mcp.marketplace.back" : "mcp.marketplace.browse")}
              </Button>
            </Show>
          </div>
          <Show when={catalog.loading}>
            <Loader />
          </Show>
          <Show when={loaded() && !catalog.loading && current() === undefined}>
            <div class="flex flex-wrap items-center gap-2" role="alert">
              <p class="text-13-regular text-v2-text-text-muted">{language.t("mcp.marketplace.loadFailed")}</p>
              <Button size="small" variant="neutral" onClick={() => void refetch()}>
                {language.t("mcp.marketplace.retry")}
              </Button>
            </div>
          </Show>
          <Show when={current()}>
            <p class="text-13-regular text-v2-text-text-muted">
              {language.t(state.browse ? "mcp.marketplace.description" : "mcp.marketplace.scope")}
            </p>
            <Show when={state.browse}>
              <TextField
                label={language.t("mcp.marketplace.search")}
                value={state.query}
                onChange={(query) => setState("query", query)}
              />
              <Button size="small" disabled={!state.query.trim() || state.query.trim().length > 100 || registry.loading}
                onClick={() => setLookup({ query: state.query.trim(), attempt: lookup.attempt + 1 })}>
                {language.t("mcp.marketplace.registry")}
              </Button>
              <Show when={registry.loading}><Loader /></Show>
              <Show when={!registry.loading && lookup.query === state.query.trim() && lookup.query}>
                <Show when={registry()} fallback={<p role="alert">{language.t("mcp.marketplace.registryFailed")}</p>}>
                  {(entries) => <>
                    <p class="text-13-regular text-v2-text-text-muted">{language.t("mcp.marketplace.registryScope")}</p>
                    <Show when={!entries().length}>{language.t("mcp.marketplace.noMatches")}</Show>
                    <For each={entries()}>{(entry) => <div class="p-3">
                      <h4 class="text-13-medium break-all">{entry.name}</h4>
                      <p class="text-13-regular">{entry.description}</p>
                      <p class="text-12-regular">{[...entry.transports, ...entry.packages.map((kind) => language.t("mcp.marketplace.package", { kind }))].join(" · ")}</p>
                    </div>}</For>
                  </>}
                </Show>
              </Show>
            </Show>
            <Show
              when={visible().length}
              fallback={
                <p class="text-13-regular text-v2-text-text-muted">
                  {language.t(state.browse ? "mcp.marketplace.noMatches" : "mcp.marketplace.empty")}
                </p>
              }
            >
              <SettingsList variant="catalog">
                <For each={visible()}>
                  {(entry) => (
                    <div class="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 p-3">
                      <div class="flex min-w-0 flex-1 basis-48 flex-col gap-1">
                        <h4 class="text-13-medium text-v2-text-text-base">{entry.name}</h4>
                        <p class="text-13-regular text-v2-text-text-muted">{entry.description}</p>
                        <p class="text-12-regular break-all text-v2-text-text-muted">
                          {language.t("mcp.marketplace.publisher", { publisher: entry.publisher, host: entry.host })}
                        </p>
                        <Show when={entry.tool}>{(tool) => <p class="text-12-regular break-all">{language.t("mcp.marketplace.tool", { tool: tool() })}</p>}</Show>
                      </div>
                      <div class="flex shrink-0 items-center gap-2">
                        <Show when={state.pending === entry.id}>
                          <Loader />
                        </Show>
                        <Show when={installed(entry.id)}>
                          <span class="text-12-regular text-v2-text-text-muted">
                            {language.t("mcp.marketplace.installed")}
                          </span>
                        </Show>
                        <Button
                          size="small"
                          variant={installed(entry.id) ? "neutral" : "contrast"}
                          aria-label={language.t(
                            installed(entry.id) ? "mcp.marketplace.removeLabel" : "mcp.marketplace.addLabel",
                            { name: entry.name },
                          )}
                          aria-busy={state.pending === entry.id}
                          disabled={!!state.pending || catalog.loading}
                          onClick={() => void change(entry.id)}
                        >
                          {language.t(installed(entry.id) ? "mcp.marketplace.remove" : "mcp.marketplace.add")}
                        </Button>
                      </div>
                    </div>
                  )}
                </For>
              </SettingsList>
            </Show>
            <Show when={state.error}>
              <p class="text-13-regular text-v2-state-fg-danger" role="alert">
                {language.t("mcp.marketplace.changeFailed")}
              </p>
            </Show>
          </Show>
        </div>
      </Show>
    </>
  )
}
