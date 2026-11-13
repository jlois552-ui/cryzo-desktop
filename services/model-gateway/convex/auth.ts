import { createClient, type GenericCtx } from "@convex-dev/better-auth"
import { convex } from "@convex-dev/better-auth/plugins"
import { betterAuth, type BetterAuthOptions } from "better-auth/minimal"
import { bearer, deviceAuthorization } from "better-auth/plugins"
import { components } from "./_generated/api"
import type { DataModel } from "./_generated/dataModel"
import authConfig from "./auth.config"
import authSchema from "./betterAuth/schema"

export const authComponent = createClient<DataModel, typeof authSchema>(components.betterAuth, { local: { schema: authSchema } })
export const createAuthOptions = (ctx: GenericCtx<DataModel>) => ({
  baseURL: process.env.SITE_URL,
  secret: process.env.BETTER_AUTH_SECRET,
  trustedOrigins: [process.env.SITE_URL!],
  database: authComponent.adapter(ctx),
  socialProviders: { google: { clientId: process.env.GOOGLE_CLIENT_ID!, clientSecret: process.env.GOOGLE_CLIENT_SECRET! } },
  session: { expiresIn: 60 * 60 * 24 * 30, updateAge: 60 * 60 * 24 },
  plugins: [
    bearer(),
    deviceAuthorization({ schema: {}, verificationUri: `${process.env.SITE_URL}/device`, expiresIn: "10m", interval: "5s", validateClient: (clientId) => clientId === "cryzo-desktop" }),
    convex({ authConfig }),
  ],
}) satisfies BetterAuthOptions
export const createAuth = (ctx: GenericCtx<DataModel>) => betterAuth(createAuthOptions(ctx))
