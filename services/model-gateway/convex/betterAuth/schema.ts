// Extend the pinned component schema for the Better Auth device authorization plugin.
import { defineSchema, defineTable } from "convex/server"
import { v } from "convex/values"
import { tables } from "./baseSchema"
export default defineSchema({
  ...tables,
  deviceCode: defineTable({ deviceCode:v.string(), userCode:v.string(), userId:v.optional(v.union(v.null(),v.string())), expiresAt:v.number(), status:v.string(), lastPolledAt:v.optional(v.union(v.null(),v.number())), pollingInterval:v.optional(v.union(v.null(),v.number())), clientId:v.optional(v.union(v.null(),v.string())), scope:v.optional(v.union(v.null(),v.string())) }).index("deviceCode",["deviceCode"]).index("userCode",["userCode"]),
})
