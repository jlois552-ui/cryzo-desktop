// Decode real Rust workflow captures with the unchanged upstream schemas.
// CRYZO_CONTRACT_CAPTURE is an optional output of the native workflow tests.
import { Schema } from "effect"
import { Session } from "@opencode/schema/session"
import { SessionMessage } from "@opencode/schema/session-message"
import { SessionEvent } from "@opencode/schema/session-event"
import { Agent } from "@opencode/schema/agent"
import { Model } from "@opencode/schema/model"
import { Provider } from "@opencode/schema/provider"
import { Integration } from "@opencode/schema/integration"

const capture = await Bun.file(process.argv[2] ?? "../../native/target/desktop-contract.json").json()
Schema.decodeUnknownSync(Session.Info)(capture.session)
for (const message of capture.messages) Schema.decodeUnknownSync(SessionMessage.Info)(message)
for (const event of capture.events) Schema.decodeUnknownSync(SessionEvent.All)(event)
for (const agent of capture.agent) Schema.decodeUnknownSync(Agent.Info)(agent)
for (const model of capture.model) Schema.decodeUnknownSync(Model.Info)(model)
for (const provider of capture.provider) Schema.decodeUnknownSync(Provider.Info)(provider)
for (const integration of capture.integration) Schema.decodeUnknownSync(Integration.Info)(integration)
console.log(
  `Validated session, ${capture.messages.length} messages, ${capture.events.length} events, and all fixture catalogs with the frozen OpenCode schemas.`,
)
