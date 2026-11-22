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
    /// The catalog built into this version of Cryzo.
    pub fn load() -> Result<Self> {
        Self::parse(include_str!(
            "../../../packages/core/src/models-dev/snapshot.txt"
        ))
    }

    /// The newest catalog saved in `directory`, else the built-in one.
    pub fn load_saved(directory: &std::path::Path) -> Result<Self> {
        match std::fs::read_to_string(directory.join(SAVED))
            .ok()
            .and_then(|text| Self::parse(&text).ok())
            .filter(Catalog::complete)
        {
            Some(saved) => Ok(saved),
            None => Self::load(),
        }
    }

    /// Fetches the live catalog, the same one OpenCode refreshes from, and saves it in
    /// `directory` for the next start. New models reach Cryzo this way without an update.
    pub async fn fetch(directory: &std::path::Path) -> Result<Self> {
        let text = reqwest::Client::new()
            .get(LIVE)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let catalog = Self::parse(&text)?;
        anyhow::ensure!(
            catalog.complete(),
            "the live model catalog looks incomplete"
        );
        let partial = directory.join(format!("{SAVED}.partial"));
        std::fs::write(&partial, &text)?;
        std::fs::rename(&partial, directory.join(SAVED))?;
        Ok(catalog)
    }

    /// A sanity check before a downloaded catalog replaces the one in use.
    fn complete(&self) -> bool {
        self.providers.len() >= 100
            && ["openai", "anthropic", "google"].iter().all(|id| {
                self.providers
                    .iter()
                    .any(|provider| provider.id == *id && !provider.models.is_empty())
            })
    }

    pub fn parse(text: &str) -> Result<Self> {
        let source: BTreeMap<String, SourceProvider> = serde_json::from_str(text)?;
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

/// The live catalog: chat models, routing decisions and image models, as OpenCode fetches it.
const LIVE: &str = "https://models.opencode.ai/experimental/v2.0/api.json?type=chat,decision,image";
/// Where the last good copy of the live catalog is kept, next to the database.
const SAVED: &str = "models.json";

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
