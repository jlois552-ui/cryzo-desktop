import { defineSchema, defineTable } from "convex/server"
import { v } from "convex/values"

export default defineSchema({
  usage: defineTable({ user: v.string(), day: v.string(), calls: v.number() }).index("by_user_and_day", ["user", "day"]),
  calls: defineTable({ user: v.string(), step: v.string(), expires: v.number(), status: v.union(v.literal("active"), v.literal("finished")) })
    .index("by_user_and_step", ["user", "step"]).index("by_status_and_expires", ["status", "expires"]),
  pool: defineTable({ key: v.string(), minute: v.number(), calls: v.number(), blockedUntil: v.optional(v.number()) }).index("by_key", ["key"]),
})
