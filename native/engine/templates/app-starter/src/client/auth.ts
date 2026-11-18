import { createAuthClient } from "better-auth/react"

// Talks to /api/auth on the same site.
export const authClient = createAuthClient()
