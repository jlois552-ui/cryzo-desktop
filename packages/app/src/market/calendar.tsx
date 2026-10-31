import { Icon } from "@opencode/ui/icon"
import { IconButton } from "@opencode/ui/icon-button"
import { Loader } from "@opencode/ui/loader"
import { For, Show, createMemo, onMount } from "solid-js"
import { useLanguage } from "@/runtime/i18n/language"
import type { Post } from "./api"
import { ChannelAvatar } from "./icons"
import { range, startOfDay, type MarketModel, type View } from "./model"

const hour = 3_600_000
const day = 24 * hour
const views: View[] = ["day", "week", "month", "list"]

export function Calendar(props: { model: MarketModel; onOpen: (post: Post) => void; onCreate: (at: number) => void }) {
  const language = useLanguage()
  const locale = () => language.intl()
  const bounds = createMemo(() => range(props.model.state.view, props.model.state.anchor))
  const days = createMemo(() => {
    const [from, to] = bounds()
    return Array.from({ length: Math.round((to - from) / day) }, (_, index) => {
      const date = new Date(from)
      date.setDate(date.getDate() + index)
      return date.getTime()
    })
  })
  const postsOn = (start: number, length: number) =>
    props.model.visiblePosts().filter((post) => post.scheduledFor >= start && post.scheduledFor < start + length)
  const title = createMemo(() => {
    const [from, to] = bounds()
    const format = new Intl.DateTimeFormat(locale(), { month: "long", day: "numeric", year: "numeric" })
    if (props.model.state.view === "day") return format.format(from)
    if (props.model.state.view === "month")
      return new Intl.DateTimeFormat(locale(), { month: "long", year: "numeric" }).format(props.model.state.anchor)
    return `${format.format(from)} – ${format.format(to - day)}`
  })

  const reschedule = async (postId: string, at: number) => {
    const post = props.model.state.posts.find((item) => item._id === postId)
    if (!post || post.status === "publishing" || post.status === "published") return
    // Month cells keep the original time of day.
    const original = new Date(post.scheduledFor)
    const target = new Date(at)
    if (props.model.state.view === "month") target.setHours(original.getHours(), original.getMinutes())
    if (target.getTime() === post.scheduledFor) return
    props.model.setState("posts", (item) => item._id === postId, "scheduledFor", target.getTime())
    const done = await props.model.run(() => props.model.api.updatePost({ postId, scheduledFor: target.getTime() }))
    if (done === undefined) void props.model.loadPosts()
  }

  const drop = (at: number) => (event: DragEvent) => {
    event.preventDefault()
    const postId = event.dataTransfer?.getData("text/cryzo-post")
    if (postId) void reschedule(postId, at)
  }

  return (
    <div class="flex min-h-0 min-w-0 flex-1 flex-col gap-4">
      <div class="flex flex-wrap items-center gap-3">
        <div class="flex h-9 items-center rounded-[8px] border border-v2-border-border-muted">
          <IconButton
            variant="ghost-muted"
            icon={<Icon name="outline-arrow-left" />}
            aria-label={language.t("market.calendar.previous")}
            onClick={() => props.model.navigate(-1)}
          />
          <button
            type="button"
            class="h-full border-x border-v2-border-border-muted px-3 text-[13px] text-v2-text-text-base hover:bg-v2-overlay-simple-overlay-hover"
            onClick={() => props.model.today()}
          >
            {language.t("market.calendar.today")}
          </button>
          <IconButton
            variant="ghost-muted"
            icon={<Icon name="outline-arrow-right" />}
            aria-label={language.t("market.calendar.next")}
            onClick={() => props.model.navigate(1)}
          />
        </div>
        <span class="text-[15px] font-medium leading-6 text-v2-text-text-base">{title()}</span>
        <Show when={props.model.state.loadingPosts}>
          <Loader class="size-4" />
        </Show>
        <div class="flex-1" />
        <Show when={props.model.state.groups.length}>
          <select
            class="h-9 rounded-[8px] border border-v2-border-border-muted bg-v2-background-bg-base px-3 text-[13px] text-v2-text-text-base"
            value={props.model.state.group ?? ""}
            aria-label={language.t("market.calendar.groupFilter")}
            onChange={(event) => props.model.setState("group", event.currentTarget.value || undefined)}
          >
            <option value="">{language.t("market.calendar.allGroups")}</option>
            <For each={props.model.state.groups}>{(group) => <option value={group._id}>{group.name}</option>}</For>
          </select>
        </Show>
        <div class="flex h-9 items-center rounded-[8px] border border-v2-border-border-muted p-0.5" role="tablist">
          <For each={views}>
            {(view) => (
              <button
                type="button"
                role="tab"
                aria-selected={props.model.state.view === view}
                class="h-full rounded-[6px] px-3 text-[13px] text-v2-text-text-muted aria-selected:bg-v2-background-bg-layer-03 aria-selected:text-v2-text-text-base"
                onClick={() => props.model.setView(view)}
              >
                {language.t(`market.calendar.view.${view}`)}
              </button>
            )}
          </For>
        </div>
      </div>

      <Show when={props.model.state.view === "week" || props.model.state.view === "day"}>
        <TimeGrid days={days()} postsOn={postsOn} drop={drop} onOpen={props.onOpen} onCreate={props.onCreate} model={props.model} />
      </Show>
      <Show when={props.model.state.view === "month"}>
        <div class="grid min-h-0 flex-1 grid-cols-7 gap-1 overflow-y-auto" data-scrollable>
          <For each={days().slice(0, 7)}>
            {(start) => (
              <div class="pb-1 text-center text-[12px] leading-4 text-v2-text-text-faint">
                {new Intl.DateTimeFormat(locale(), { weekday: "short" }).format(start)}
              </div>
            )}
          </For>
          <For each={days()}>
            {(start) => (
              <div
                class="flex min-h-[112px] flex-col gap-1 rounded-[8px] bg-v2-background-bg-layer-01 p-1.5 data-[faded]:opacity-50"
                data-faded={new Date(start).getMonth() !== new Date(props.model.state.anchor).getMonth() ? "" : undefined}
                onDragOver={(event) => event.preventDefault()}
                onDrop={drop(start)}
                onDblClick={() => props.onCreate(start + 9 * hour)}
              >
                <span class="text-[12px] leading-4 text-v2-text-text-muted data-[today]:font-semibold data-[today]:text-v2-text-text-accent" data-today={isToday(start) ? "" : undefined}>
                  {new Date(start).getDate()}
                </span>
                <For each={postsOn(start, day).slice(0, 3)}>{(post) => <PostCard post={post} model={props.model} onOpen={props.onOpen} compact />}</For>
                <Show when={postsOn(start, day).length > 3}>
                  <span class="px-1 text-[11px] leading-4 text-v2-text-text-faint">
                    {language.t("market.calendar.overflow", { count: postsOn(start, day).length - 3 })}
                  </span>
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>
      <Show when={props.model.state.view === "list"}>
        <div class="min-h-0 flex-1 overflow-y-auto" data-scrollable>
          <Show
            when={props.model.visiblePosts().length}
            fallback={<p class="py-10 text-center text-[13px] text-v2-text-text-muted">{language.t("market.calendar.empty")}</p>}
          >
            <For each={days().filter((start) => postsOn(start, day).length)}>
              {(start) => (
                <section class="mb-5">
                  <h3 class="mb-2 text-[13px] font-medium leading-5 text-v2-text-text-muted">
                    {new Intl.DateTimeFormat(locale(), { weekday: "long", month: "long", day: "numeric" }).format(start)}
                  </h3>
                  <div class="flex flex-col gap-2">
                    <For each={postsOn(start, day).sort((a, b) => a.scheduledFor - b.scheduledFor)}>
                      {(post) => <PostCard post={post} model={props.model} onOpen={props.onOpen} />}
                    </For>
                  </div>
                </section>
              )}
            </For>
          </Show>
        </div>
      </Show>
    </div>
  )
}

function TimeGrid(props: {
  model: MarketModel
  days: number[]
  postsOn: (start: number, length: number) => Post[]
  drop: (at: number) => (event: DragEvent) => void
  onOpen: (post: Post) => void
  onCreate: (at: number) => void
}) {
  const language = useLanguage()
  let scroller: HTMLDivElement | undefined
  // Open at the working day rather than midnight.
  onMount(() => requestAnimationFrame(() => scroller?.scrollTo({ top: 8 * 96 })))
  const columns = () => `72px repeat(${props.days.length}, minmax(140px, 1fr))`

  return (
    <div class="min-h-0 flex-1 overflow-auto rounded-[10px] border border-v2-border-border-muted" data-scrollable ref={scroller}>
      <div class="sticky top-0 z-10 grid bg-v2-background-bg-base" style={{ "grid-template-columns": columns() }}>
        <div />
        <For each={props.days}>
          {(start) => (
            <div class="flex flex-col items-center border-b border-s border-v2-border-border-muted py-2">
              <span class="text-[12px] leading-4 text-v2-text-text-faint">
                {new Intl.DateTimeFormat(language.intl(), { weekday: "long" }).format(start)}
              </span>
              <span
                class="text-[14px] font-medium leading-5 text-v2-text-text-base data-[today]:text-v2-text-text-accent"
                data-today={isToday(start) ? "" : undefined}
              >
                {new Intl.DateTimeFormat(language.intl(), { day: "numeric", month: "long" }).format(start)}
              </span>
            </div>
          )}
        </For>
      </div>
      <div class="grid" style={{ "grid-template-columns": columns() }}>
        <For each={Array.from({ length: 24 }, (_, index) => index)}>
          {(slot) => (
            <>
              <div class="h-24 pe-2 pt-1 text-end text-[12px] leading-4 text-v2-text-text-faint">
                {new Intl.DateTimeFormat(language.intl(), { hour: "numeric" }).format(new Date(2026, 0, 1, slot))}
              </div>
              <For each={props.days}>
                {(start) => {
                  const at = () => start + slot * hour
                  return (
                    <div
                      class="flex h-24 flex-col gap-1 overflow-y-auto border-s border-t border-v2-border-border-muted p-1 hover:bg-v2-overlay-simple-overlay-hover data-[past]:bg-v2-background-bg-layer-01"
                      data-past={at() + hour < Date.now() ? "" : undefined}
                      data-scrollable
                      onDragOver={(event) => event.preventDefault()}
                      onDrop={props.drop(at())}
                      onClick={(event) => {
                        if (event.target === event.currentTarget && at() + hour > Date.now()) props.onCreate(Math.max(at(), Date.now() + 15 * 60_000))
                      }}
                    >
                      <For each={props.postsOn(at(), hour)}>{(post) => <PostCard post={post} model={props.model} onOpen={props.onOpen} compact />}</For>
                    </div>
                  )
                }}
              </For>
            </>
          )}
        </For>
      </div>
    </div>
  )
}

export function PostCard(props: { post: Post; model: MarketModel; onOpen: (post: Post) => void; compact?: boolean }) {
  const language = useLanguage()
  const group = () => props.model.group(props.post.groupId)
  const accounts = () =>
    props.post.targets.flatMap((target) => {
      const account = props.model.account(target.connectedAccountId)
      return account ? [account] : []
    })
  const movable = () => props.post.status !== "publishing" && props.post.status !== "published"

  return (
    <button
      type="button"
      draggable={movable()}
      class="flex w-full shrink-0 flex-col overflow-hidden rounded-[8px] bg-v2-background-bg-layer-02 text-start hover:bg-v2-background-bg-layer-03 data-[status=draft]:opacity-70"
      data-status={props.post.status}
      onDragStart={(event) => event.dataTransfer?.setData("text/cryzo-post", props.post._id)}
      onClick={() => props.onOpen(props.post)}
    >
      <Show when={group()}>
        {(group) => (
          <span class="truncate px-2 text-center text-[11px] font-medium leading-4 text-white" style={{ "background-color": group().color }}>
            {group().name}
          </span>
        )}
      </Show>
      <span class="flex items-center gap-1.5 px-1.5 py-1">
        <span class="flex shrink-0 -space-x-2">
          <For each={accounts().slice(0, 3)}>{(account) => <ChannelAvatar account={account} size="small" />}</For>
        </span>
        <span class="min-w-0 flex-1 truncate text-[12px] leading-4 text-v2-text-text-base">
          {props.post.content || language.t("market.post.untitled")}
        </span>
        <span
          class="size-1.5 shrink-0 rounded-full"
          classList={{
            "bg-v2-icon-icon-faint": props.post.status === "draft",
            "bg-v2-icon-icon-accent": props.post.status === "scheduled" || props.post.status === "publishing",
            "bg-v2-state-fg-success": props.post.status === "published",
            "bg-v2-state-fg-danger": props.post.status === "failed",
          }}
          title={language.t(`market.status.${props.post.status}`)}
        />
      </span>
      <Show when={!props.compact}>
        <span class="px-2 pb-1.5 text-[11px] leading-4 text-v2-text-text-faint">
          {new Intl.DateTimeFormat(language.intl(), { hour: "numeric", minute: "2-digit" }).format(props.post.scheduledFor)} ·{" "}
          {language.t(`market.status.${props.post.status}`)}
        </span>
      </Show>
    </button>
  )
}

function isToday(start: number) {
  return start === startOfDay(Date.now())
}
