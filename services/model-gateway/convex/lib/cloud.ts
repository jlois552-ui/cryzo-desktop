// Cloud workspace limits, like Replit's free tier: a small daily allowance per user and a hard
// monthly budget for the whole service, so sandbox spending can never run away. Time is counted
// as wall-clock running minutes, which is never less than what Cloudflare bills for the container.

export type Caps = {
  /** Free running minutes per user per UTC day. */
  dailyMinutes: number
  /** Running minutes for all users together per UTC month; new sessions stop when it is spent. */
  monthlyMinutes: number
}

export function caps(env: Record<string, string | undefined> = process.env): Caps {
  const number = (value: string | undefined, fallback: number) => {
    const parsed = Number(value)
    return Number.isFinite(parsed) && parsed >= 0 ? parsed : fallback
  }
  return {
    dailyMinutes: number(env.CLOUD_FREE_DAILY_MINUTES, 30),
    monthlyMinutes: number(env.CLOUD_MONTHLY_MINUTES, 600),
  }
}

/** Minutes a user may still run now, or why they may not. */
export function allowance(input: { usedToday: number; usedMonth: number; caps: Caps }) {
  const monthly = input.caps.monthlyMinutes - input.usedMonth
  if (monthly <= 0) return { ok: false as const, reason: "Cloud workspaces are paused for everyone until next month. Local projects still work." }
  const daily = input.caps.dailyMinutes - input.usedToday
  if (daily <= 0) return { ok: false as const, reason: "You've used today's free cloud time. It resets at midnight UTC; local projects still work." }
  return { ok: true as const, minutes: Math.min(daily, monthly) }
}

export const day = (now: number) => new Date(now).toISOString().slice(0, 10)
export const month = (now: number) => new Date(now).toISOString().slice(0, 7)

/** One workspace per user and project, with a name that is safe in a hostname. */
export async function sandboxName(user: string, project: string) {
  const bytes = new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(`${user}\0${project}`)))
  return `cryzo-${Array.from(bytes.slice(0, 12), (byte) => byte.toString(16).padStart(2, "0")).join("")}`
}
