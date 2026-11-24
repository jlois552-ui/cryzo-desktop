use crate::remote;
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Results {
    results: Vec<Source>,
}

#[derive(Deserialize)]
struct Source {
    title: String,
    url: String,
    snippet: String,
}

pub async fn search(query: &str) -> Result<Value> {
    let query = query.trim();
    ensure!(
        !query.is_empty() && query.chars().count() <= 400 && query.split_whitespace().count() <= 50,
        "Use a web search query between 1 and 400 characters, at most 50 words."
    );
    let data = remote::call(
        reqwest::Method::POST,
        "search/web",
        Some(remote::account().context("Sign in to Cryzo to use web search.")?),
        json!({"query":query}),
    )
    .await?;
    let results: Results =
        serde_json::from_value(data).context("Cryzo returned invalid search results.")?;
    Ok(Value::String(if results.results.is_empty() {
        "No web results found.".into()
    } else {
        results
            .results
            .into_iter()
            .map(|source| {
                format!(
                    "Title: {}\nURL: {}\nSnippet: {}",
                    source.title, source.url, source.snippet
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }))
}
