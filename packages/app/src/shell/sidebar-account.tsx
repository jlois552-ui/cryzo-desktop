import { useDialog } from "@opencode/ui/context/dialog"
import { Icon } from "@opencode/ui/icon"
import { IconButton } from "@opencode/ui/icon-button"
import { Show, createMemo, onMount } from "solid-js"
import { createStore } from "solid-js/store"
import { useLanguage } from "@/runtime/i18n/language"
import { usePlatform } from "@/runtime/platform/platform"
import { authTokenFromCredentials } from "@/runtime/server/api"
import { useServerSDK } from "@/runtime/server/client"
import { ServerProvider } from "@/runtime/server/current"
import { useGlobal } from "@/runtime/server/runtime"
import { useSettingsDialog } from "@/settings/command"

/** The sidebar's footer, as in Cline: the Cryzo account, then Settings. */
export function SidebarAccount() {
  const global = useGlobal()
  const openSettings = useSettingsDialog()
  const language = useLanguage()
  // The account lives in the local Cryzo engine; other servers have none.
  const sidecar = createMemo(() => global.servers.list().find((item) => item.type === "sidecar"))

  return (
    <div class="flex h-11 w-full shrink-0 items-center gap-2">
      <div class="min-w-0 flex-1">
        <Show when={sidecar()} keyed>
          {(conn) => (
            <ServerProvider conn={conn}>
              <Account onOpen={() => openSettings()} />
            </ServerProvider>
          )}
        </Show>
      </div>
      <IconButton
        variant="ghost-muted"
        icon={<Icon name="settings-gear" />}
        aria-label={language.t("sidebar.settings")}
        title={language.t("sidebar.settings")}
        onClick={() => openSettings()}
      />
    </div>
  )
}

function Account(props: { onOpen: () => void }) {
  const sdk = useServerSDK()
  const platform = usePlatform()
  const language = useLanguage()
  const dialog = useDialog()
  const [state, setState] = createStore({ loaded: false, signedIn: false, name: "", email: "" })

  const load = async () => {
    const password = sdk.http.password
    const response = await (platform.fetch ?? fetch)(`${sdk.url}/api/cryzo/account`, {
      headers: password ? { Authorization: `Basic ${authTokenFromCredentials({ password })}` } : {},
    }).catch(() => undefined)
    const data = (await response?.json().catch(() => undefined)) as
      | { signedIn?: boolean; name?: string | null; email?: string | null }
      | undefined
    setState({ loaded: true, signedIn: !!data?.signedIn, name: data?.name ?? "", email: data?.email ?? "" })
  }
  onMount(() => void load())

  const signIn = async () => {
    const { DialogConnectProvider, useProviderConnectController } = await import("@/providers/connect/dialog")
    const controller = useProviderConnectController()
    controller.select("cryzo")
    void dialog.show(() => <DialogConnectProvider controller={controller} standalone onConnected={() => void load()} />)
  }

  const name = () => state.name || state.email || language.t("sidebar.account.signedIn")

  return (
    <Show when={state.loaded}>
      <button
        type="button"
        data-action="sidebar-account"
        class="flex h-10 w-full min-w-0 items-center gap-2 rounded-[8px] px-1.5 text-start hover:bg-v2-overlay-simple-overlay-hover"
        onClick={() => (state.signedIn ? props.onOpen() : void signIn())}
      >
        <span class="grid size-7 shrink-0 place-items-center rounded-full bg-v2-background-bg-layer-03 text-[12px] font-medium leading-4 text-v2-text-text-base">
          <Show when={state.signedIn} fallback={<Icon name="plus" size="small" />}>
            {name().slice(0, 1).toUpperCase()}
          </Show>
        </span>
        <span class="min-w-0 flex-1">
          <span class="block truncate text-[13px] leading-4 text-v2-text-text-base">
            {state.signedIn ? name() : language.t("sidebar.account.signIn")}
          </span>
          <span class="block truncate text-[12px] leading-4 text-v2-text-text-faint">
            {language.t(state.signedIn ? "sidebar.account.plan" : "sidebar.account.signInHint")}
          </span>
        </span>
      </button>
    </Show>
  )
}
