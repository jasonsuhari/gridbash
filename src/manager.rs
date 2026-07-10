use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::ManagerConfig;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagerDecision {
    Continue(String),
    Done(String),
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    content: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DecisionPayload {
    status: String,
    #[serde(default)]
    message: String,
}

pub fn review(config: &ManagerConfig, goal: &str, pane_output: &str) -> Result<ManagerDecision> {
    config.validate()?;
    let client = Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .context("failed to create manager API client")?;
    let response = client
        .post(config.endpoint.trim())
        .bearer_auth(config.api_key.trim())
        .json(&request_body(&config.model, goal, pane_output))
        .send()
        .context("manager API request failed")?;
    let status = response.status();
    let body = response
        .text()
        .context("failed to read manager API response")?;
    if !status.is_success() {
        return Err(anyhow!(
            "manager API returned {status}: {}",
            truncate_error(&body)
        ));
    }

    let response: ChatResponse =
        serde_json::from_str(&body).context("manager API returned invalid JSON")?;
    let content = response
        .choices
        .first()
        .map(|choice| choice.message.content.trim())
        .filter(|content| !content.is_empty())
        .ok_or_else(|| anyhow!("manager API returned no message"))?;
    parse_decision(content)
}

pub(crate) fn request_body(model: &str, goal: &str, pane_output: &str) -> serde_json::Value {
    json!({
        "model": model.trim(),
        "temperature": 0.2,
        "messages": [
            {
                "role": "system",
                "content": "You manage exactly one terminal pane. Never mention, address, or route work to any other pane. Decide whether the pane has completed its goal. Return JSON only: {\"status\":\"continue\",\"message\":\"one concise instruction for this same pane\"} or {\"status\":\"done\",\"message\":\"short completion summary\"}."
            },
            {
                "role": "user",
                "content": format!("GOAL:\n{goal}\n\nLATEST OUTPUT FROM THIS PANE ONLY:\n{pane_output}")
            }
        ]
    })
}

pub(crate) fn parse_decision(content: &str) -> Result<ManagerDecision> {
    let content = content.trim();
    let content = content
        .strip_prefix("```json")
        .or_else(|| content.strip_prefix("```"))
        .unwrap_or(content)
        .strip_suffix("```")
        .unwrap_or(content)
        .trim();
    let payload: DecisionPayload =
        serde_json::from_str(content).context("manager response was not a decision object")?;
    let message = payload.message.trim().to_string();
    match payload.status.trim().to_ascii_lowercase().as_str() {
        "continue" if !message.is_empty() => Ok(ManagerDecision::Continue(message)),
        "done" => Ok(ManagerDecision::Done(message)),
        "continue" => Err(anyhow!("manager continue decision had no instruction")),
        status => Err(anyhow!("unknown manager decision status '{status}'")),
    }
}

fn truncate_error(value: &str) -> String {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    value.chars().take(240).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_is_explicitly_pane_local() {
        let body = request_body("gpt-test", "ship it", "tests passed");
        let messages = body["messages"].as_array().expect("messages");
        let system = messages[0]["content"].as_str().expect("system prompt");
        assert!(system.contains("exactly one terminal pane"));
        assert!(system.contains("Never mention, address, or route work to any other pane"));
        assert_eq!(body["model"], "gpt-test");
    }

    #[test]
    fn parses_continue_and_done_decisions() {
        assert_eq!(
            parse_decision(r#"{"status":"continue","message":"run tests"}"#).unwrap(),
            ManagerDecision::Continue("run tests".into())
        );
        assert_eq!(
            parse_decision("```json\n{\"status\":\"done\",\"message\":\"shipped\"}\n```").unwrap(),
            ManagerDecision::Done("shipped".into())
        );
    }
}
