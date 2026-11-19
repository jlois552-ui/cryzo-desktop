// Daily dictation seconds per user, so the mic can't run up the Workers AI bill.
import { v } from "convex/values"
import { internalMutation } from "../_generated/server"
import { DAILY_SECONDS } from "../lib/voice"

/** Records `seconds` of dictation for today, or refuses when it would pass the daily allowance. */
export const spend = internalMutation({
  args: { user: v.string(), day: v.string(), seconds: v.number() },
  handler: async (ctx, args) => {
    const row = await ctx.db
      .query("voiceUsage")
      .withIndex("by_user_and_day", (q) => q.eq("user", args.user).eq("day", args.day))
      .unique()
    const used = row?.seconds ?? 0
    if (used + args.seconds > DAILY_SECONDS)
      throw new Error(`You've used today's ${DAILY_SECONDS / 60} minutes of dictation. It resets tomorrow (UTC).`)
    if (row) await ctx.db.patch(row._id, { seconds: used + args.seconds })
    else await ctx.db.insert("voiceUsage", { user: args.user, day: args.day, seconds: args.seconds })
  },
})
