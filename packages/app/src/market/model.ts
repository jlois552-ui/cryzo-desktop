import { createStore, reconcile } from "solid-js/store"
import { showToast } from "@/shell/notifications/toast"
import { useLanguage } from "@/runtime/i18n/language"
import { MarketError, useMarketApi, type Account, type Group, type Media, type Post } from "./api"

export type Section = "calendar" | "analytics" | "media"
export type View = "day" | "week" | "month" | "list"

const day = 86_400_000

export function startOfDay(time: number) {
  const date = new Date(time)
  date.setHours(0, 0, 0, 0)
  return date.getTime()
}

/** Weeks start on Monday, as in Postiz. */
export function startOfWeek(time: number) {
  const date = new Date(startOfDay(time))
  date.setDate(date.getDate() - ((date.getDay() + 6) % 7))
  return date.getTime()
}

/** The visible [from, to) range for a view around `anchor`. Month views cover whole weeks. */
export function range(view: View, anchor: number): [number, number] {
  if (view === "day") return [startOfDay(anchor), startOfDay(anchor) + day]
  if (view === "week") return [startOfWeek(anchor), startOfWeek(anchor) + 7 * day]
  if (view === "list") return [startOfDay(anchor), startOfDay(anchor) + 30 * day]
  const first = new Date(anchor)
  first.setDate(1)
  const last = new Date(first)
  last.setMonth(last.getMonth() + 1)
  const from = startOfWeek(first.getTime())
  return [from, from + Math.ceil((startOfDay(last.getTime()) - from) / (7 * day)) * 7 * day]
}

export function shift(view: View, anchor: number, direction: 1 | -1) {
  const date = new Date(anchor)
  if (view === "day") date.setDate(date.getDate() + direction)
  else if (view === "week") date.setDate(date.getDate() + 7 * direction)
  else if (view === "list") date.setDate(date.getDate() + 30 * direction)
  else date.setMonth(date.getMonth() + direction)
  return date.getTime()
}

export function createMarketModel() {
  const api = useMarketApi()
  const language = useLanguage()
  const [state, setState] = createStore({
    section: "calendar" as Section,
    view: "week" as View,
    anchor: Date.now(),
    group: undefined as string | undefined,
    collapsed: false,
    ready: false,
    signIn: false,
    loadingPosts: false,
    accounts: [] as Account[],
    groups: [] as Group[],
    posts: [] as Post[],
    media: [] as Media[],
  })

  /** Runs a request; a missing sign-in switches the screen to the sign-in prompt instead of a toast. */
  const run = async <T>(work: () => Promise<T>): Promise<T | undefined> => {
    try {
      const result = await work()
      setState("signIn", false)
      return result
    } catch (error) {
      if (error instanceof MarketError && error.signIn) {
        setState("signIn", true)
        return
      }
      showToast({
        variant: "error",
        title: language.t("market.toast.failed"),
        description: error instanceof Error && error.message ? error.message : language.t("common.requestFailed"),
      })
    }
  }

  const loadAccounts = async (sync = false) => {
    const result = await run(() => (sync ? api.sync() : api.accounts()))
    if (!result) return
    setState("accounts", reconcile(result.accounts, { key: "_id" }))
    setState("groups", reconcile(result.groups, { key: "_id" }))
  }

  const loadPosts = async () => {
    const [from, to] = range(state.view, state.anchor)
    setState("loadingPosts", true)
    const result = await run(() => api.posts(from, to))
    setState("loadingPosts", false)
    if (result) setState("posts", reconcile(result, { key: "_id" }))
  }

  const loadMedia = async () => {
    const result = await run(() => api.media())
    if (result) setState("media", reconcile(result, { key: "_id" }))
  }

  const load = async () => {
    // Composio is the source of truth for connections; sync on open picks up channels added elsewhere.
    await loadAccounts(true)
    if (!state.signIn) await Promise.all([loadPosts(), loadMedia()])
    setState("ready", true)
  }

  return {
    api,
    state,
    setState,
    run,
    load,
    loadAccounts,
    loadPosts,
    loadMedia,
    navigate(direction: 1 | -1) {
      setState("anchor", shift(state.view, state.anchor, direction))
      void loadPosts()
    },
    today() {
      setState("anchor", Date.now())
      void loadPosts()
    },
    setView(view: View) {
      setState("view", view)
      void loadPosts()
    },
    account(connectedAccountId: string) {
      return state.accounts.find((item) => item.connectedAccountId === connectedAccountId)
    },
    group(groupId: string | undefined) {
      return groupId ? state.groups.find((item) => item._id === groupId) : undefined
    },
    visiblePosts() {
      return state.group ? state.posts.filter((post) => post.groupId === state.group) : state.posts
    },
  }
}

export type MarketModel = ReturnType<typeof createMarketModel>
