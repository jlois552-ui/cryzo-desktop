/** Cryzo's computer-use tools: the desktop and its Chrome, and the app preview in the browser pane. */
export function isComputerTool(name: string) {
  return name.startsWith("desktop_") || name.startsWith("page_") || name.startsWith("preview")
}
