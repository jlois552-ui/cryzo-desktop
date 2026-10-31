import { Button } from "@opencode/ui/button"
import { Icon } from "@opencode/ui/icon"
import { For, Show, createMemo } from "solid-js"
import { createStore } from "solid-js/store"
import { useLanguage } from "@/runtime/i18n/language"
import { usePlatform } from "@/runtime/platform/platform"
import { channels, type Channel, type Post } from "./api"
import { PostCard } from "./calendar"
import { networkName } from "./channels"
import { NetworkBadge } from "./icons"
import { startOfDay, type MarketModel } from "./model"

const statuses = ["draft", "scheduled", "publishing", "published", "failed"] as const

/** Wix's Social posts: a seven-day planner to create and publish, and the list of every post with its status. */
export function Posts(props: { model: MarketModel; onPlan: () => void; onOpen: (post: Post) => void; onCreate: (at: number) => void }) {
  const language = useLanguage()
  const platform = usePlatform()
  const [state, setState] = createStore({
    tab: "planner" as "planner" | "list",
    status: "" as "" | (typeof statuses)[number],
    channel: "" as "" | Channel,
  })
  // Calendar days, not 24-hour steps, so a daylight-saving change does not repeat or skip a day.
  const week = Array.from({ length: 7 }, (_, index) => {
    const date = new Date(startOfDay(Date.now()))
    date.setDate(date.getDate() + index)
    return date.getTime()
  })
  const next = (start: number) => {
    const date = new Date(start)
    date.setDate(date.getDate() + 1)
    return date.getTime()
  }
  const on = (start: number) =>
    props.model.state.feed.filter((post) => post.scheduledFor >= start && post.scheduledFor < next(start)).sort((a, b) => a.scheduledFor - b.scheduledFor)
  const rows = createMemo(() =>
    props.model.state.feed
      .filter((post) => !state.status || post.status === state.status)
      .filter((post) => !state.channel || post.targets.some((target) => target.channel === state.channel))
      .sort((a, b) => b.scheduledFor - a.scheduledFor),
  )
  const link = (post: Post) => post.deliveries.find((delivery) => delivery.remoteUrl)?.remoteUrl
  const field = "h-8 rounded-[8px] border border-v2-border-border-base bg-v2-background-bg-base px-2 text-[13px] text-v2-text-text-base"

  return (
    <div class="flex min-h-0 flex-col gap-4">
      <div class="flex items-center gap-1 border-b border-v2-border-border-muted" role="tablist">
        <For each={["planner", "list"] as const}>
          {(tab) => (
            <button
              type="button"
              role="tab"
              aria-selected={state.tab === tab}
              class="-mb-px h-9 border-b-2 border-transparent px-3 text-[13px] text-v2-text-text-muted aria-selected:border-v2-icon-icon-accent aria-selected:text-v2-text-text-base"
              onClick={() => setState("tab", tab)}
            >
              {language.t(`market.posts.tab.${tab}`)}
            </button>
          )}
        </For>
      </div>

      <Show when={state.tab === "planner"}>
        <div class="flex items-center gap-3">
          <p class="min-w-0 flex-1 text-[13px] leading-5 text-v2-text-text-muted">{language.t("market.posts.plannerDescription")}</p>
          <Button variant="neutral" disabled={!props.model.state.accounts.length} onClick={props.onPlan}>
            {language.t("market.overview.ideas.action")}
          </Button>
        </div>
        <div class="grid gap-2 [grid-template-columns:repeat(auto-fit,minmax(130px,1fr))]">
          <For each={week}>
            {(start) => (
              <div class="flex min-h-[220px] flex-col gap-1.5 rounded-[10px] bg-v2-background-bg-layer-01 p-2">
                <div class="px-1 text-[12px] leading-4 text-v2-text-text-muted">
                  {new Intl.DateTimeFormat(language.intl(), { weekday: "long" }).format(start)}
                </div>
                <div class="px-1 text-[18px] font-medium leading-6 text-v2-text-text-base">{new Date(start).getDate()}</div>
                <For each={on(start)}>{(post) => <PostCard post={post} model={props.model} onOpen={props.onOpen} />}</For>
                <button
                  type="button"
                  class="mt-auto grid h-8 place-items-center rounded-[8px] text-v2-icon-icon-muted hover:bg-v2-overlay-simple-overlay-hover"
                  aria-label={language.t("market.channels.create")}
                  onClick={() => props.onCreate(Math.max(start + 9 * 3_600_000, Date.now() + 3_600_000))}
                >
                  <Icon name="plus" />
                </button>
              </div>
            )}
          </For>
        </div>
      </Show>

      <Show when={state.tab === "list"}>
        <div class="flex flex-wrap items-center gap-2">
          <select class={field} value={state.status} aria-label={language.t("market.posts.filterStatus")} onChange={(event) => setState("status", event.currentTarget.value as typeof state.status)}>
            <option value="">{language.t("market.posts.allStatuses")}</option>
            <For each={statuses}>{(status) => <option value={status}>{language.t(`market.status.${status}`)}</option>}</For>
          </select>
          <select class={field} value={state.channel} aria-label={language.t("market.posts.filterChannel")} onChange={(event) => setState("channel", event.currentTarget.value as typeof state.channel)}>
            <option value="">{language.t("market.posts.allChannels")}</option>
            <For each={channels}>{(channel) => <option value={channel}>{networkName(language, channel)}</option>}</For>
          </select>
        </div>
        <div class="overflow-hidden rounded-[10px] border border-v2-border-border-muted">
          <div class="grid grid-cols-[minmax(0,1fr)_120px_170px_110px_32px] gap-3 border-b border-v2-border-border-muted bg-v2-background-bg-layer-01 px-3 py-2 text-[12px] leading-4 text-v2-text-text-muted">
            <span>{language.t("market.posts.column.content")}</span>
            <span>{language.t("market.posts.column.channels")}</span>
            <span>{language.t("market.posts.column.date")}</span>
            <span>{language.t("market.posts.column.status")}</span>
            <span />
          </div>
          <For each={rows()} fallback={<p class="px-3 py-6 text-center text-[13px] leading-5 text-v2-text-text-muted">{language.t("market.posts.empty")}</p>}>
            {(post) => (
              <div class="grid grid-cols-[minmax(0,1fr)_120px_170px_110px_32px] items-center gap-3 border-b border-v2-border-border-muted px-3 py-2 last:border-b-0 hover:bg-v2-overlay-simple-overlay-hover">
                <button type="button" class="truncate text-start text-[13px] leading-5 text-v2-text-text-base" onClick={() => props.onOpen(post)}>
                  {post.content || language.t("market.post.untitled")}
                </button>
                <span class="flex gap-1">
                  <For each={[...new Set(post.targets.map((target) => target.channel))]}>{(channel) => <NetworkBadge channel={channel} />}</For>
                </span>
                <span class="text-[12px] leading-4 text-v2-text-text-muted">
                  {new Intl.DateTimeFormat(language.intl(), { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" }).format(post.scheduledFor)}
                </span>
                <span class="text-[12px] leading-4 text-v2-text-text-base">{language.t(`market.status.${post.status}`)}</span>
                <Show when={link(post)}>
                  {(url) => (
                    <button
                      type="button"
                      class="grid size-7 place-items-center rounded-[6px] text-v2-icon-icon-muted hover:bg-v2-overlay-simple-overlay-hover"
                      aria-label={language.t("market.posts.open")}
                      title={language.t("market.posts.open")}
                      onClick={() => platform.openExternal(url())}
                    >
                      <Icon name="outline-arrow-up-right" />
                    </button>
                  )}
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  )
}
