/** Marketing's pages, in sidebar order. Shared by the app sidebar and the Marketing route without loading Marketing. */
export const sections = ["calendar", "analytics", "media"] as const
export type Section = (typeof sections)[number]
