use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct Catalog {
    pub upstream_commit: String,
    pub providers: Vec<Provider>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub credential_env: Vec<String>,
    pub models: Vec<Model>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub apis: Vec<Api>,
    pub details: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Api {
    pub protocol: String,
    pub base_url: Option<String>,
    #[serde(default)]
    pub compatibility: serde_json::Value,
}

impl Catalog {
    pub fn native_api(&self, provider: &str, model: &str) -> Result<&Api> {
        let model = self
            .providers
            .iter()
            .find(|p| p.id == provider)
            .and_then(|p| p.models.iter().find(|m| m.id == model))
            .ok_or_else(|| anyhow::anyhow!("unknown model for this provider"))?;
        [
            "responses",
            "messages",
            "generate-content",
            "chat-completions",
        ]
        .iter()
        .find_map(|protocol| {
            model
                .apis
                .iter()
                .find(|api| api.protocol == *protocol && api.base_url.is_some())
        })
        .ok_or_else(|| {
            anyhow::anyhow!("this model's native transport has not passed the compatibility gate")
        })
    }
    pub fn load() -> Result<Self> {
        let source: BTreeMap<String, SourceProvider> = serde_json::from_str(include_str!(
            "../../../packages/core/src/models-dev/snapshot.txt"
        ))?;
        Ok(Self {
            upstream_commit: "caebbcc0807e630629725f320f35a09aa4a427be".into(),
            providers: source
                .into_values()
                .map(|provider| Provider {
                    id: provider.id,
                    name: provider.name,
                    credential_env: provider.env,
                    models: provider
                        .models
                        .into_values()
                        .map(|model| Model {
                            id: model.id,
                            name: model.name,
                            details: serde_json::to_value(model.details).unwrap_or_default(),
                            apis: model
                                .api
                                .into_iter()
                                .map(|(protocol, api)| Api {
                                    protocol,
                                    base_url: api.base_url,
                                    compatibility: api.compatibility,
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        })
    }

    pub fn chat_endpoint(&self, provider: &str, model: &str) -> Result<&str> {
        // The legacy direct-provider API must never send a Cryzo account token to NVIDIA.
        if provider == "cryzo" {
            bail!("Cryzo Free requires the desktop gateway transport")
        }
        let Some(provider) = self.providers.iter().find(|entry| entry.id == provider) else {
            bail!("unknown provider")
        };
        let Some(model) = provider.models.iter().find(|entry| entry.id == model) else {
            bail!("unknown model for this provider")
        };
        // Catalog presence is not parity. Only explicitly implemented transports may run.
        model
            .apis
            .iter()
            .find(|api| api.protocol == "chat-completions")
            .and_then(|api| api.base_url.as_deref())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "this model's native transport has not passed the compatibility gate"
                )
            })
    }
}

#[derive(Deserialize)]
struct SourceProvider {
    id: String,
    name: String,
    #[serde(default)]
    env: Vec<String>,
    models: BTreeMap<String, SourceModel>,
}
#[derive(Deserialize)]
struct SourceModel {
    id: String,
    name: String,
    #[serde(default)]
    api: BTreeMap<String, SourceApi>,
    #[serde(flatten)]
    details: BTreeMap<String, serde_json::Value>,
}
#[derive(Deserialize)]
struct SourceApi {
    base_url: Option<String>,
    #[serde(default)]
    compatibility: serde_json::Value,
}
