import { readFileSync, writeFileSync } from "node:fs"
import { createHash } from "node:crypto"

const revision = "caebbcc0807e630629725f320f35a09aa4a427be"
const source = readFileSync(new URL("../packages/core/src/models-dev/snapshot.txt", import.meta.url))
const providers = Object.values(JSON.parse(source)).map((provider) => ({
  id: provider.id,
  name: provider.name,
  credential_env: provider.env ?? [],
  models: Object.values(provider.models).map((model) => ({
    id: model.id,
    name: model.name,
    apis: Object.entries(model.api ?? {}).map(([protocol, api]) => ({ protocol, base_url: api.base_url ?? null })),
  })).sort((a, b) => a.id.localeCompare(b.id)),
})).sort((a, b) => a.id.localeCompare(b.id))

writeFileSync(new URL("provider-inventory.json", import.meta.url), JSON.stringify(providers.map((provider) => ({
  id: provider.id,
  name: provider.name,
  models: provider.models.length,
  protocols: [...new Set(provider.models.flatMap((model) => model.apis.map((api) => api.protocol)))].sort(),
  auth_verified: false,
  transport_verified: false,
  tools_verified: false,
})), null, 2) + "\n")
writeFileSync(new URL("baseline.json", import.meta.url), JSON.stringify({
  repository: "https://github.com/anomalyco/opencode",
  commit: revision,
  catalog_sha256: createHash("sha256").update(source).digest("hex"),
  providers: providers.length,
  models: providers.reduce((total, provider) => total + provider.models.length, 0),
  release_gate: "Every baseline provider's authentication, transport, tool calls, cancellation and recovery must pass before public alpha.",
}, null, 2) + "\n")
