/** Marketing's pages, in sidebar order. Shared by the app sidebar and the Marketing route without loading Marketing. */
export const sections = ["overview", "posts", "calendar", "analytics", "media", "channels"] as const
export type Section = (typeof sections)[number]
