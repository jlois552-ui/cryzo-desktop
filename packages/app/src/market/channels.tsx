import { Button } from "@opencode/ui/button"
import { useDialog } from "@opencode/ui/context/dialog"
import { Dialog, DialogTitle } from "@opencode/ui/dialog"
import { Icon } from "@opencode/ui/icon"
import { IconButton } from "@opencode/ui/icon-button"
import { Loader } from "@opencode/ui/loader"
import { For, Show, onCleanup, onMount } from "solid-js"
import { createStore } from "solid-js/store"
import { useLanguage } from "@/runtime/i18n/language"
import { usePlatform } from "@/runtime/platform/platform"
import { channels, type Account, type Channel } from "./api"
import { ChannelAvatar, NetworkBadge } from "./icons"
import type { MarketModel } from "./model"

/** Wix's "Manage accounts": every network with its connected accounts and what each needs next. */
export function ChannelsPage(props: { model: MarketModel }) {
  const language = useLanguage()
  const dialog = useDialog()
  const accounts = (channel: Channel) => props.model.state.accounts.filter((account) => account.channel === channel)

  return (
    <div class="grid gap-3 [grid-template-columns:repeat(auto-fill,minmax(300px,1fr))]">
      <For each={channels}>
        {(channel) => {
          const provider = () => props.model.provider(channel)
          return (
            <section class="flex flex-col gap-3 rounded-[12px] border border-v2-border-border-muted p-4">
              <div class="flex items-center gap-3">
                <NetworkBadge channel={channel} class="size-9 text-[14px] leading-none" />
                <div class="min-w-0 flex-1">
                  <div class="text-[14px] font-medium leading-5 text-v2-text-text-base">{networkName(language, channel)}</div>
                  <div class="text-[12px] leading-4 text-v2-text-text-muted">
                    {language.t(
                      !provider()?.configured
                        ? "market.channels.notSetUp"
                        : provider()?.pending
                          ? "market.channels.pending"
                          : "market.channels.ready",
                    )}
                  </div>
                </div>
                <Button
                  variant="neutral"
                  size="small"
                  icon="plus"
                  disabled={!provider()?.configured}
                  onClick={() => dialog.show(() => <ConnectDialog model={props.model} channel={channel} />)}
                >
                  {language.t("market.channels.connect")}
                </Button>
              </div>
              <For each={accounts(channel)}>
                {(account) => (
                  <div class="flex items-center gap-3 rounded-[8px] bg-v2-background-bg-layer-01 px-2 py-2">
                    <ChannelAvatar account={account} size="small" />
                    <div class="min-w-0 flex-1">
                      <div class="truncate text-[13px] leading-5 text-v2-text-text-base">{account.name}</div>
                      <Show
                        when={account.needsReconnect || account.needsTarget}
                        fallback={
                          <Show when={account.targetName && account.targetName !== account.name}>
                            <div class="truncate text-[12px] leading-4 text-v2-text-text-muted">{account.targetName}</div>
                          </Show>
                        }
                      >
                        <div class="truncate text-[12px] leading-4 text-v2-state-fg-warning">
                          {language.t(account.needsReconnect ? "market.channels.reconnectNeeded" : "market.channels.targetMissing")}
                        </div>
                      </Show>
                    </div>
                    <Show when={account.needsReconnect}>
                      <Button
                        variant="ghost"
                        size="small"
                        onClick={() => dialog.show(() => <ConnectDialog model={props.model} channel={channel} reconnect={account.connectedAccountId} />)}
                      >
                        {language.t("market.channels.reconnect")}
                      </Button>
                    </Show>
                    <IconButton
                      variant="ghost-muted"
                      icon={<Icon name="outline-dots" />}
                      aria-label={language.t("market.channels.settings", { name: account.name })}
                      onClick={() => dialog.show(() => <ChannelSettingsDialog model={props.model} account={account} />)}
                    />
                  </div>
                )}
              </For>
            </section>
          )
        }}
      </For>
    </div>
  )
}

export const networkName = (language: ReturnType<typeof useLanguage>, channel: Channel) => language.t(`market.network.${channel}`)

/** Opens the network's sign-in in the browser and waits for the account to appear (or, on reconnect, to recover). */
export function ConnectDialog(props: { model: MarketModel; channel: Channel; reconnect?: string }) {
  const language = useLanguage()
  const platform = usePlatform()
  const dialog = useDialog()
  const [state, setState] = createStore({ waiting: false })
  const known = new Set(props.model.state.accounts.map((account) => account.connectedAccountId))
  const done = () =>
    props.model.state.accounts.some((account) =>
      props.reconnect ? account.connectedAccountId === props.reconnect && !account.needsReconnect : !known.has(account.connectedAccountId),
    )
  let timer: ReturnType<typeof setInterval> | undefined
  onCleanup(() => clearInterval(timer))

  const start = async () => {
    const url = await props.model.run(() => props.model.api.connect(props.channel))
    if (!url) return
    platform.openExternal(url)
    setState("waiting", true)
    clearInterval(timer)
    let attempts = 0
    timer = setInterval(async () => {
      attempts++
      await props.model.loadAccounts()
      if (!done() && attempts < 60) return
      clearInterval(timer)
      if (done()) dialog.close()
      else setState("waiting", false)
    }, 5000)
  }
  onMount(() => void start())

  return (
    <Dialog fit containerClass="!w-[min(calc(100vw_-_32px),420px)] !rounded-xl">
      <div class="flex flex-col items-center gap-3 p-6 text-center">
        <NetworkBadge channel={props.channel} class="size-10 text-[15px] leading-none" />
        <DialogTitle>
          <span class="text-[16px] font-medium leading-6 text-v2-text-text-base">
            {language.t("market.connect.title", { network: networkName(language, props.channel) })}
          </span>
        </DialogTitle>
        <p class="text-[13px] leading-5 text-v2-text-text-muted">{language.t("market.connect.description")}</p>
        <Show
          when={state.waiting}
          fallback={
            <Button variant="contrast" onClick={() => void start()}>
              {language.t("market.connect.again")}
            </Button>
          }
        >
          <div class="flex items-center gap-2 text-[13px] leading-5 text-v2-text-text-base">
            <Loader class="size-4" />
            {language.t("market.connect.waiting")}
          </div>
          <Button variant="ghost" onClick={() => void props.model.loadAccounts()}>
            {language.t("market.channels.refresh")}
          </Button>
        </Show>
      </div>
    </Dialog>
  )
}

function ChannelSettingsDialog(props: { model: MarketModel; account: Account }) {
  const language = useLanguage()
  const dialog = useDialog()
  const targeted = ["facebook", "instagram", "linkedin"].includes(props.account.channel)
  const [state, setState] = createStore({
    name: props.account.name,
    groupId: props.account.groupId ?? "",
    targetId: props.account.targetId ?? "",
    newGroup: "",
    targets: undefined as { id: string; name: string }[] | undefined,
    saving: false,
    confirm: false,
  })

  onMount(async () => {
    if (!targeted || props.account.needsReconnect) return
    const targets = await props.model.run(() => props.model.api.targets(props.account.connectedAccountId))
    setState("targets", targets ?? [])
  })

  const save = async () => {
    setState("saving", true)
    const groupId = state.newGroup.trim()
      ? await props.model.run(() => props.model.api.saveGroup({ name: state.newGroup, color: palette[props.model.state.groups.length % palette.length] }))
      : state.groupId || null
    const saved = await props.model.run(async () => {
      await props.model.api.updateAccount({ connectedAccountId: props.account.connectedAccountId, name: state.name, groupId: groupId ?? null })
      if (state.targetId && state.targetId !== props.account.targetId)
        await props.model.api.chooseTarget(props.account.connectedAccountId, state.targetId)
      return null
    })
    setState("saving", false)
    if (saved === undefined) return
    await props.model.loadAccounts()
    dialog.close()
  }

  const disconnect = async () => {
    setState("saving", true)
    const done = await props.model.run(() => props.model.api.disconnect(props.account.connectedAccountId))
    setState("saving", false)
    if (done === undefined) return
    await props.model.loadAccounts()
    dialog.close()
  }

  const field = "h-9 w-full rounded-[8px] border border-v2-border-border-base bg-v2-background-bg-base px-3 text-[13px] text-v2-text-text-base"

  return (
    <Dialog fit containerClass="!w-[min(calc(100vw_-_32px),440px)] !rounded-xl">
      <div class="flex flex-col gap-4 p-6">
        <div class="flex items-center gap-3">
          <ChannelAvatar account={props.account} />
          <DialogTitle>
            <span class="text-[16px] font-medium leading-6 text-v2-text-text-base">{props.account.name}</span>
          </DialogTitle>
        </div>
        <label class="flex flex-col gap-1.5 text-[12px] leading-4 text-v2-text-text-muted">
          {language.t("market.channels.name")}
          <input class={field} value={state.name} maxLength={100} onInput={(event) => setState("name", event.currentTarget.value)} />
        </label>
        <Show when={targeted && !props.account.needsReconnect}>
          <label class="flex flex-col gap-1.5 text-[12px] leading-4 text-v2-text-text-muted">
            {language.t(`market.channels.target.${props.account.channel as "facebook" | "instagram" | "linkedin"}`)}
            <Show when={state.targets} fallback={<Loader class="size-4" />}>
              {(targets) => (
                <Show
                  when={targets().length}
                  fallback={<span class="text-[13px] leading-5 text-v2-state-fg-warning">{language.t("market.channels.noTargets")}</span>}
                >
                  <select class={field} value={state.targetId} onChange={(event) => setState("targetId", event.currentTarget.value)}>
                    <option value="">{language.t("market.channels.choose")}</option>
                    <For each={targets()}>{(target) => <option value={target.id}>{target.name}</option>}</For>
                  </select>
                </Show>
              )}
            </Show>
          </label>
        </Show>
        <label class="flex flex-col gap-1.5 text-[12px] leading-4 text-v2-text-text-muted">
          {language.t("market.channels.group")}
          <select class={field} value={state.groupId} onChange={(event) => setState("groupId", event.currentTarget.value)}>
            <option value="">{language.t("market.channels.ungrouped")}</option>
            <For each={props.model.state.groups}>{(group) => <option value={group._id}>{group.name}</option>}</For>
          </select>
          <input
            class={field}
            placeholder={language.t("market.channels.newGroup")}
            value={state.newGroup}
            maxLength={60}
            onInput={(event) => setState("newGroup", event.currentTarget.value)}
          />
        </label>
        <div class="mt-2 flex items-center gap-2">
          <Show
            when={state.confirm}
            fallback={
              <Button variant="ghost" class="text-v2-state-fg-danger" onClick={() => setState("confirm", true)}>
                {language.t("market.channels.disconnect")}
              </Button>
            }
          >
            <Button variant="ghost" class="text-v2-state-fg-danger" disabled={state.saving} onClick={() => void disconnect()}>
              {language.t("market.channels.disconnectConfirm")}
            </Button>
          </Show>
          <div class="flex-1" />
          <Button variant="ghost" onClick={() => dialog.close()}>
            {language.t("common.cancel")}
          </Button>
          <Button variant="contrast" disabled={state.saving || !state.name.trim()} onClick={() => void save()}>
            {language.t("common.save")}
          </Button>
        </div>
      </div>
    </Dialog>
  )
}

export const palette = ["#3B82F6", "#8B5CF6", "#EC4899", "#F59E0B", "#10B981", "#EF4444", "#06B6D4"]
