import { initializeNativeDesktop } from "./native"

await initializeNativeDesktop()
// Mount the real upstream desktop renderer and its existing components/styles.
await import("../../desktop/src/renderer/index")
window.postMessage("desktop-rpc-port", "*", [window.cryzoPort])
