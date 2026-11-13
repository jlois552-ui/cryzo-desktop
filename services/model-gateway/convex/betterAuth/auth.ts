import { createAuth } from "../auth"
// Used only by Better Auth's schema generator, never imported by runtime code.
export const auth = createAuth({} as Parameters<typeof createAuth>[0])
