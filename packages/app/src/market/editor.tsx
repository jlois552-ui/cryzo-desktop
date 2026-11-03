import { Button } from "@opencode/ui/button"
import { useDialog } from "@opencode/ui/context/dialog"
import { Dialog, DialogTitle } from "@opencode/ui/dialog"
import { Loader } from "@opencode/ui/loader"
import { Icon } from "@opencode/ui/icon"
import { For, Show, createEffect, createMemo, onCleanup } from "solid-js"
import { createStore } from "solid-js/store"
import { useLanguage } from "@/runtime/i18n/language"
import { usePlatform } from "@/runtime/platform/platform"
import { showToast } from "@/shell/notifications/toast"
import { channels, type Account, type Channel, type Options, type Post, type Target } from "./api"
import { ConnectDialog, networkName } from "./channels"
import { ChannelAvatar, NetworkBadge } from "./icons"
import type { MarketModel } from "./model"

const limits: Partial<Record<Channel, number>> = {
  x: 280,
  linkedin: 3000,
  instagram: 2200,
  tiktok: 2200,
  youtube: 5000,
  facebook: 63206,
  reddit: 40000,
}

function localInput(time: number) {
  const date = new Date(time)
  date.setMinutes(date.getMinutes() - date.getTimezoneOffset())
  return date.toISOString().slice(0, 16)
}

export function PostEditor(props: {
  model: MarketModel
  post?: Post
  at?: number
  content?: string
  targets?: Target[]
}) {
  const language = useLanguage()
  const dialog = useDialog()
  const platform = usePlatform()
  const locked = props.post?.status === "publishing" || props.post?.status === "published"
  const [state, setState] = createStore({
    content: props.post?.content ?? props.content ?? "",
    targets: (props.post?.targets ?? props.targets ?? []) as Target[],
    editing: "" as string,
    mediaIds: (props.post?.mediaIds ?? []) as string[],
    options: { ...(props.post?.options ?? {}) } as Options,
    groupId: props.post?.groupId ?? "",
    when: localInput(props.post?.scheduledFor ?? props.at ?? Date.now() + 3_600_000),
    library: false,
    uploading: false,
    saving: false,
    confirmDelete: false,
  })

  const selected = (account: Account) =>
    state.targets.some((target) => target.connectedAccountId === account.connectedAccountId)
  const usable = (account: Account) => !account.needsReconnect && !account.needsTarget
  const toggle = (account: Account) => {
    if (locked || (!usable(account) && !selected(account))) return
    if (selected(account)) {
      setState("targets", (targets) =>
        targets.filter((target) => target.connectedAccountId !== account.connectedAccountId),
      )
      if (state.editing === account.connectedAccountId) setState("editing", "")
      return
    }
    setState("targets", (targets) => [
      ...targets,
      { channel: account.channel, connectedAccountId: account.connectedAccountId },
    ])
  }
  const used = createMemo(() => new Set(state.targets.map((target) => target.channel)))
  const current = () => state.targets.find((target) => target.connectedAccountId === state.editing)
  const text = () => (current() ? (current()!.content ?? state.content) : state.content)
  const setText = (value: string) => {
    if (!current()) return setState("content", value)
    setState("targets", (target) => target.connectedAccountId === state.editing, "content", value)
  }
  const limit = () => {
    const channel = current()?.channel
    if (channel) return limits[channel]
    return used().has("x") ? 280 : undefined
  }
  const media = () =>
    state.mediaIds.flatMap((id) => {
      const item = props.model.state.media.find((entry) => entry.storageId === id)
      return item ? [item] : []
    })

  const upload = async () => {
    const paths = await platform.openFilePathPickerDialog?.({
      title: language.t("market.editor.upload"),
      multiple: true,
      extensions: ["png", "jpg", "jpeg", "gif", "webp", "mp4", "mov", "webm"],
    })
    if (!paths?.length) return
    setState("uploading", true)
    for (const path of paths) {
      const result = await props.model.run(() => props.model.api.upload(path))
      if (result) setState("mediaIds", (ids) => [...ids, result.storageId])
    }
    await props.model.loadMedia()
    setState("uploading", false)
  }

  const save = async (mode: "draft" | "scheduled" | "now") => {
    const scheduledFor = mode === "now" ? Date.now() : new Date(state.when).getTime()
    if (mode === "scheduled" && scheduledFor < Date.now()) {
      showToast({ variant: "error", title: language.t("market.editor.pastTime") })
      return
    }
    setState("saving", true)
    const fields = {
      content: state.content,
      targets: state.targets.map((target) => ({
        ...target,
        ...(target.content?.trim() ? {} : { content: undefined }),
      })),
      scheduledFor,
      status: mode === "draft" ? ("draft" as const) : ("scheduled" as const),
      mediaIds: state.mediaIds,
      options: state.options,
    }
    const postId = props.post
      ? await props.model.run(async () => {
          await props.model.api.updatePost({ postId: props.post!._id, ...fields, groupId: state.groupId || null })
          return props.post!._id
        })
      : await props.model.run(() =>
          props.model.api.createPost({
            ...fields,
            requestKey: crypto.randomUUID(),
            ...(state.groupId ? { groupId: state.groupId } : {}),
          }),
        )
    const published = postId && mode === "now" ? await props.model.run(() => props.model.api.publish(postId)) : null
    setState("saving", false)
    if (!postId || published === undefined) return
    showToast({
      title: language.t(
        mode === "now"
          ? "market.toast.published"
          : mode === "draft"
            ? "market.toast.draftSaved"
            : "market.toast.scheduled",
      ),
    })
    await props.model.loadPosts()
    dialog.close()
  }

  const remove = async () => {
    setState("saving", true)
    const done = await props.model.run(() => props.model.api.deletePost(props.post!._id))
    setState("saving", false)
    if (done === undefined) return
    await props.model.loadPosts()
    dialog.close()
  }

  const field =
    "h-9 w-full rounded-[8px] border border-v2-border-border-base bg-v2-background-bg-base px-3 text-[13px] text-v2-text-text-base"
  const label = "flex flex-col gap-1.5 text-[12px] leading-4 text-v2-text-text-muted"

  return (
    <Dialog fit containerClass="!w-[min(calc(100vw_-_32px),1080px)] !max-h-[calc(100vh_-_48px)] !rounded-xl">
      <div class="flex max-h-[calc(100vh_-_48px)] min-h-0 flex-col">
        <div class="border-b border-v2-border-border-muted px-6 py-4">
          <DialogTitle>
            <span class="text-[18px] font-medium leading-7 text-v2-text-text-base">
              {language.t(props.post ? "market.editor.titleEdit" : "market.editor.titleNew")}
            </span>
          </DialogTitle>
        </div>
        <div class="flex min-h-0 flex-1">
          <div class="flex min-h-0 min-w-0 flex-1 flex-col gap-5 overflow-y-auto px-6 py-5" data-scrollable>
            <section class="flex flex-col gap-2 rounded-[12px] border border-v2-border-border-muted p-3">
              <div class="flex items-center gap-2">
                <span class="text-[13px] font-medium leading-5 text-v2-text-text-base">
                  {language.t("market.editor.postTo")}
                </span>
                <div class="flex-1" />
                <Show when={!locked}>
                  <span class="text-[12px] leading-4 text-v2-text-text-muted">
                    {language.t("market.editor.connectMore")}
                  </span>
                  <For each={channels.filter((channel) => props.model.provider(channel)?.configured)}>
                    {(channel) => (
                      <button
                        type="button"
                        class="rounded-full"
                        aria-label={language.t("market.connect.title", { network: networkName(language, channel) })}
                        title={language.t("market.connect.title", { network: networkName(language, channel) })}
                        onClick={() => dialog.show(() => <ConnectDialog model={props.model} channel={channel} />)}
                      >
                        <NetworkBadge channel={channel} class="size-5 text-[9px] leading-none" />
                      </button>
                    )}
                  </For>
                </Show>
              </div>
              <div class="grid gap-1 [grid-template-columns:repeat(auto-fill,minmax(220px,1fr))]">
                <For each={props.model.state.accounts}>
                  {(account) => (
                    <button
                      type="button"
                      role="switch"
                      class="flex h-11 items-center gap-2.5 rounded-[8px] px-2 text-start hover:bg-v2-overlay-simple-overlay-hover disabled:cursor-default disabled:hover:bg-transparent"
                      aria-checked={selected(account)}
                      disabled={locked || (!usable(account) && !selected(account))}
                      onClick={() => toggle(account)}
                    >
                      <ChannelAvatar account={account} size="small" />
                      <span class="min-w-0 flex-1">
                        <span class="block truncate text-[13px] leading-5 text-v2-text-text-base">{account.name}</span>
                        <Show when={!usable(account)}>
                          <span class="block truncate text-[11px] leading-4 text-v2-state-fg-warning">
                            {language.t(
                              account.needsReconnect
                                ? "market.channels.reconnectNeeded"
                                : "market.channels.targetMissing",
                            )}
                          </span>
                        </Show>
                      </span>
                      <span
                        class="flex h-4 w-7 shrink-0 items-center rounded-full bg-v2-background-bg-layer-03 p-0.5 transition-colors"
                        classList={{ "!bg-v2-background-bg-accent": selected(account) }}
                      >
                        <span
                          class="size-3 rounded-full bg-v2-background-bg-base transition-transform"
                          classList={{ "translate-x-3": selected(account) }}
                        />
                      </span>
                    </button>
                  )}
                </For>
              </div>
            </section>

            <div class="flex flex-col gap-2">
              <div class="flex flex-wrap gap-1" role="tablist">
                <button
                  type="button"
                  role="tab"
                  aria-selected={!state.editing}
                  class="h-7 rounded-[6px] px-2.5 text-[12px] text-v2-text-text-muted aria-selected:bg-v2-background-bg-layer-03 aria-selected:text-v2-text-text-base"
                  onClick={() => setState("editing", "")}
                >
                  {language.t("market.editor.global")}
                </button>
                <For each={state.targets}>
                  {(target) => (
                    <button
                      type="button"
                      role="tab"
                      aria-selected={state.editing === target.connectedAccountId}
                      class="flex h-7 items-center gap-1.5 rounded-[6px] px-2.5 text-[12px] text-v2-text-text-muted aria-selected:bg-v2-background-bg-layer-03 aria-selected:text-v2-text-text-base"
                      onClick={() => setState("editing", target.connectedAccountId)}
                    >
                      <NetworkBadge channel={target.channel} class="size-3.5 text-[7px] leading-none" />
                      {props.model.account(target.connectedAccountId)?.name ?? networkName(language, target.channel)}
                    </button>
                  )}
                </For>
              </div>
              <textarea
                class="min-h-[180px] w-full resize-y rounded-[10px] border border-v2-border-border-base bg-v2-background-bg-base p-3 text-[14px] leading-5 text-v2-text-text-base"
                placeholder={language.t(current() ? "market.editor.overridePlaceholder" : "market.editor.placeholder")}
                value={text()}
                readOnly={locked}
                onInput={(event) => setText(event.currentTarget.value)}
              />
              <div class="flex justify-between text-[12px] leading-4 text-v2-text-text-faint">
                <span>{current() ? language.t("market.editor.overrideHint") : ""}</span>
                <Show when={limit()}>
                  {(max) => (
                    <span classList={{ "text-v2-state-fg-danger": text().length > max() }}>
                      {language.t("market.editor.characters", { count: text().length, limit: max() })}
                    </span>
                  )}
                </Show>
              </div>
            </div>

            <div class="flex flex-col gap-2">
              <div class="flex items-center gap-2">
                <span class="text-[13px] font-medium leading-5 text-v2-text-text-base">
                  {language.t("market.editor.media")}
                </span>
                <div class="flex-1" />
                <Button
                  variant="ghost"
                  size="small"
                  disabled={locked}
                  onClick={() => setState("library", !state.library)}
                >
                  {language.t("market.editor.library")}
                </Button>
                <Button
                  variant="neutral"
                  size="small"
                  icon="plus"
                  disabled={locked || state.uploading}
                  onClick={() => void upload()}
                >
                  {language.t("market.editor.upload")}
                </Button>
                <Show when={state.uploading}>
                  <Loader class="size-4" />
                </Show>
              </div>
              <div class="flex flex-wrap gap-2">
                <For each={media()}>
                  {(item) => (
                    <button
                      type="button"
                      class="relative size-20 overflow-hidden rounded-[8px] bg-v2-background-bg-layer-02"
                      title={language.t("market.editor.removeMedia")}
                      disabled={locked}
                      onClick={() => setState("mediaIds", (ids) => ids.filter((id) => id !== item.storageId))}
                    >
                      <MediaThumb model={props.model} url={item.url} type={item.contentType} />
                    </button>
                  )}
                </For>
              </div>
              <Show when={state.library}>
                <div
                  class="grid max-h-56 grid-cols-6 gap-2 overflow-y-auto rounded-[8px] bg-v2-background-bg-layer-01 p-2"
                  data-scrollable
                >
                  <For
                    each={props.model.state.media}
                    fallback={
                      <span class="col-span-6 text-[12px] text-v2-text-text-muted">
                        {language.t("market.media.empty")}
                      </span>
                    }
                  >
                    {(item) => (
                      <button
                        type="button"
                        class="aspect-square overflow-hidden rounded-[6px] ring-2 ring-transparent aria-pressed:ring-v2-icon-icon-accent"
                        aria-pressed={state.mediaIds.includes(item.storageId)}
                        onClick={() =>
                          setState("mediaIds", (ids) =>
                            ids.includes(item.storageId)
                              ? ids.filter((id) => id !== item.storageId)
                              : [...ids, item.storageId],
                          )
                        }
                      >
                        <MediaThumb model={props.model} url={item.url} type={item.contentType} />
                      </button>
                    )}
                  </For>
                </div>
              </Show>
            </div>

            <div class="grid grid-cols-2 gap-4">
              <label class={label}>
                {language.t("market.editor.when")}
                <input
                  type="datetime-local"
                  class={field}
                  value={state.when}
                  disabled={locked}
                  onInput={(event) => setState("when", event.currentTarget.value)}
                />
              </label>
              <label class={label}>
                {language.t("market.channels.group")}
                <select
                  class={field}
                  value={state.groupId}
                  disabled={locked}
                  onChange={(event) => setState("groupId", event.currentTarget.value)}
                >
                  <option value="">{language.t("market.channels.ungrouped")}</option>
                  <For each={props.model.state.groups}>
                    {(group) => <option value={group._id}>{group.name}</option>}
                  </For>
                </select>
              </label>
              <Show when={used().has("reddit")}>
                <label class={label}>
                  {language.t("market.editor.redditCommunity")}
                  <input
                    class={field}
                    placeholder="r/"
                    value={state.options.redditCommunity ?? ""}
                    disabled={locked}
                    onInput={(event) => setState("options", "redditCommunity", event.currentTarget.value)}
                  />
                </label>
              </Show>
              <Show when={used().has("youtube")}>
                <label class={label}>
                  {language.t("market.editor.youtubeTitle")}
                  <input
                    class={field}
                    maxLength={100}
                    value={state.options.youtubeTitle ?? ""}
                    disabled={locked}
                    onInput={(event) => setState("options", "youtubeTitle", event.currentTarget.value)}
                  />
                </label>
                <label class={label}>
                  {language.t("market.editor.youtubePrivacy")}
                  <select
                    class={field}
                    value={state.options.youtubePrivacy ?? "public"}
                    disabled={locked}
                    onChange={(event) => setState("options", "youtubePrivacy", event.currentTarget.value)}
                  >
                    <option value="public">{language.t("market.editor.privacy.public")}</option>
                    <option value="unlisted">{language.t("market.editor.privacy.unlisted")}</option>
                    <option value="private">{language.t("market.editor.privacy.private")}</option>
                  </select>
                </label>
              </Show>
              <Show when={used().has("tiktok")}>
                <label class={label}>
                  {language.t("market.editor.tiktokPrivacy")}
                  <select
                    class={field}
                    value={state.options.tiktokPrivacy ?? "SELF_ONLY"}
                    disabled={locked}
                    onChange={(event) => setState("options", "tiktokPrivacy", event.currentTarget.value)}
                  >
                    <option value="SELF_ONLY">{language.t("market.editor.tiktok.self")}</option>
                    <option value="MUTUAL_FOLLOW_FRIENDS">{language.t("market.editor.tiktok.friends")}</option>
                    <option value="PUBLIC_TO_EVERYONE">{language.t("market.editor.tiktok.public")}</option>
                  </select>
                </label>
              </Show>
              <Show when={used().has("instagram")}>
                <label class={label}>
                  {language.t("market.editor.instagramType")}
                  <select
                    class={field}
                    value={state.options.instagramPostType ?? "post"}
                    disabled={locked}
                    onChange={(event) =>
                      setState(
                        "options",
                        "instagramPostType",
                        event.currentTarget.value as Options["instagramPostType"],
                      )
                    }
                  >
                    <For each={["post", "reel", "story", "carousel"] as const}>
                      {(kind) => <option value={kind}>{language.t(`market.editor.instagram.${kind}`)}</option>}
                    </For>
                  </select>
                </label>
              </Show>
            </div>

            <Show when={props.post?.deliveries.length}>
              <div class="flex flex-col gap-2">
                <span class="text-[13px] font-medium leading-5 text-v2-text-text-base">
                  {language.t("market.editor.deliveries")}
                </span>
                <For each={props.post!.deliveries}>
                  {(delivery) => (
                    <div class="flex items-start gap-2 rounded-[8px] bg-v2-background-bg-layer-01 px-3 py-2">
                      <NetworkBadge channel={delivery.channel} class="mt-0.5 size-4 text-[8px] leading-none" />
                      <div class="min-w-0 flex-1">
                        <div class="text-[13px] leading-5 text-v2-text-text-base">
                          {language.t(`market.delivery.${delivery.status}`)}
                        </div>
                        <Show when={delivery.error}>
                          <div class="text-[12px] leading-4 text-v2-text-text-muted">{delivery.error}</div>
                        </Show>
                      </div>
                      <Show when={delivery.status === "failed"}>
                        <Button
                          variant="ghost"
                          size="small"
                          onClick={async () => {
                            if ((await props.model.run(() => props.model.api.retry(delivery._id))) === undefined) return
                            await props.model.loadPosts()
                            dialog.close()
                          }}
                        >
                          {language.t("market.editor.retry")}
                        </Button>
                      </Show>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          </div>

          <aside
            class="hidden w-[340px] shrink-0 flex-col gap-3 overflow-y-auto border-s border-v2-border-border-muted bg-v2-background-bg-layer-01 px-5 py-5 lg:flex"
            data-scrollable
          >
            <span class="text-[13px] font-medium leading-5 text-v2-text-text-muted">
              {language.t("market.editor.preview")}
            </span>
            <For
              each={state.targets.length ? state.targets : []}
              fallback={<p class="text-[12px] text-v2-text-text-faint">{language.t("market.editor.noChannels")}</p>}
            >
              {(target) => {
                const account = () => props.model.account(target.connectedAccountId)
                return (
                  <div class="rounded-[10px] bg-v2-background-bg-base p-3">
                    <div class="mb-2 flex items-center gap-2">
                      <Show when={account()}>{(item) => <ChannelAvatar account={item()} size="small" />}</Show>
                      <span class="truncate text-[13px] font-medium leading-5 text-v2-text-text-base">
                        {account()?.name}
                      </span>
                    </div>
                    <p class="whitespace-pre-wrap break-words text-[13px] leading-5 text-v2-text-text-base">
                      {target.content?.trim() || state.content}
                    </p>
                    <Show when={media()[0]}>
                      {(item) => (
                        <div class="mt-2 aspect-video overflow-hidden rounded-[8px] bg-v2-background-bg-layer-02">
                          <MediaThumb model={props.model} url={item().url} type={item().contentType} />
                        </div>
                      )}
                    </Show>
                  </div>
                )
              }}
            </For>
          </aside>
        </div>

        <div class="flex items-center gap-2 border-t border-v2-border-border-muted px-6 py-4">
          <Show when={props.post && props.post.status !== "publishing"}>
            <Show
              when={state.confirmDelete}
              fallback={
                <Button variant="ghost" class="text-v2-state-fg-danger" onClick={() => setState("confirmDelete", true)}>
                  {language.t("market.editor.delete")}
                </Button>
              }
            >
              <Button
                variant="ghost"
                class="text-v2-state-fg-danger"
                disabled={state.saving}
                onClick={() => void remove()}
              >
                {language.t("market.editor.deleteConfirm")}
              </Button>
            </Show>
          </Show>
          <div class="flex-1" />
          <Show when={state.saving}>
            <Loader class="size-4" />
          </Show>
          <Show when={!locked}>
            <Button variant="ghost" disabled={state.saving || !state.targets.length} onClick={() => void save("draft")}>
              {language.t("market.editor.saveDraft")}
            </Button>
            <Button variant="neutral" disabled={state.saving || !state.targets.length} onClick={() => void save("now")}>
              {language.t("market.editor.publishNow")}
            </Button>
            <Button
              variant="contrast"
              disabled={state.saving || !state.targets.length}
              onClick={() => void save("scheduled")}
            >
              {language.t(props.post ? "market.editor.update" : "market.editor.schedule")}
            </Button>
          </Show>
          <Show when={locked}>
            <Button variant="contrast" onClick={() => dialog.close()}>
              {language.t("common.close")}
            </Button>
          </Show>
        </div>
      </div>
    </Dialog>
  )
}

/** Images load through the engine as `blob:` URLs; videos show a marker instead of downloading the whole file. */
export function MediaThumb(props: { model: MarketModel; url: string | null; type: string }) {
  const [state, setState] = createStore({ src: undefined as string | undefined })
  createEffect(() => {
    const url = props.url
    if (!url || !props.type.startsWith("image/")) return
    let current: string | undefined
    let live = true
    props.model.api
      .file(url)
      .then((src) => {
        if (live) setState("src", (current = src))
        else URL.revokeObjectURL(src)
      })
      .catch(() => undefined)
    onCleanup(() => {
      live = false
      if (current) URL.revokeObjectURL(current)
    })
  })
  return (
    <Show
      when={props.type.startsWith("image/") && state.src}
      fallback={
        <span class="grid size-full place-items-center bg-v2-background-bg-layer-03 text-v2-icon-icon-muted">
          <Show when={props.type.startsWith("video/")}>
            <Icon name="outline-video" />
          </Show>
        </span>
      }
    >
      {(src) => <img src={src()} alt="" class="size-full object-cover" />}
    </Show>
  )
}
