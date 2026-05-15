use crate::Config;
use anyhow::Result;
use serde_json::json;

/// Send a chat completion request to the configured LLM provider.
pub fn ask_llm(config: &Config, system_prompt: &str, user_message: &str) -> Result<String> {
    let provider = &config.llm.active_provider;
    let providers = &config.llm.providers;

    let pc = match provider.as_str() {
        "ollama" => &providers.ollama,
        "ollama_cloud" => &providers.ollama_cloud,
        "openai" => &providers.openai,
        "openrouter" => &providers.openrouter,
        "groq" => &providers.groq,
        _ => anyhow::bail!("Unknown LLM provider: {}", provider),
    };

    if pc.api_key.is_empty() && provider != "ollama" {
        anyhow::bail!("{} API key not configured. Add it in Settings.", provider);
    }

    let base_url = pc.base_url.trim_end_matches('/');
    let url = format!("{}/v1/chat/completions", base_url);

    let body = json!({
        "model": pc.model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_message}
        ],
        "temperature": 0.3,
        "max_tokens": 2048,
    });

    let mut req = ureq::post(&url);
    if !pc.api_key.is_empty() {
        req = req.header("Authorization", &format!("Bearer {}", pc.api_key));
    }

    let resp = req
        .header("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| anyhow::anyhow!("LLM request failed: {}", e))?;

    let body_str = resp.into_body().read_to_string()
        .map_err(|e| anyhow::anyhow!("Failed to read LLM response: {}", e))?;

    let data: serde_json::Value = serde_json::from_str(&body_str)
        .map_err(|e| anyhow::anyhow!("Failed to parse LLM response: {}", e))?;

    let content = data["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("LLM returned no content: {:?}", data))?
        .to_string();

    Ok(content)
}
