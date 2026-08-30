use serde_json::{json, Value};
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatSite {
    ChatGpt,
    Gemini,
    GoogleAi,
}

impl ChatSite {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChatGpt => "chatgpt",
            Self::Gemini => "gemini",
            Self::GoogleAi => "google_ai",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ChatGpt => "ChatGPT",
            Self::Gemini => "Gemini",
            Self::GoogleAi => "Google AI Mode",
        }
    }

    pub fn wait_spec(self) -> Value {
        match self {
            Self::ChatGpt => json!({
                "until": "stable",
                "selector": "[data-message-author-role=\"assistant\"]",
                "text_not_includes": "Working",
                "settle_ms": 1200
            }),
            Self::Gemini => json!({
                "until": "stable",
                "selector": "message-content, [data-test-id=\"model-response\"], model-response",
                "settle_ms": 1500
            }),
            Self::GoogleAi => json!({
                "until": "stable",
                "selector": "[role=\"article\"], [data-conversation-id]",
                "settle_ms": 1500
            }),
        }
    }
}

pub fn supported_sites_help() -> &'static str {
    "chatgpt.com, chat.openai.com, gemini.google.com, or google.com/search?udm=50"
}

pub fn detect(url: &str) -> Option<ChatSite> {
    let Ok(parsed) = Url::parse(url) else {
        return None;
    };
    if parsed.scheme() != "https" {
        return None;
    }
    let Some(host) = parsed.host_str() else {
        return None;
    };
    let host = host.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    match host {
        "chatgpt.com" | "chat.openai.com" => Some(ChatSite::ChatGpt),
        "gemini.google.com" => Some(ChatSite::Gemini),
        "google.com"
            if parsed
                .query_pairs()
                .any(|(key, value)| key == "udm" && value == "50") =>
        {
            Some(ChatSite::GoogleAi)
        }
        _ => None,
    }
}

pub fn send_script(prompt: &str) -> String {
    json!({
        "cmd": "chat",
        "method": "fill",
        "prompt": prompt
    })
    .to_string()
}

pub fn click_send_script() -> String {
    json!({ "cmd": "chat", "method": "send" }).to_string()
}

pub fn read_script() -> String {
    json!({ "cmd": "chat", "method": "read" }).to_string()
}

pub fn ready_script() -> String {
    json!({ "cmd": "chat", "method": "ready" }).to_string()
}

pub fn parse_read(value: &Value) -> Value {
    json!({
        "answer_text": value.get("answer_text").cloned().unwrap_or(json!("")),
        "citations": value.get("citations").cloned().unwrap_or(json!([])),
        "status": if value.get("sending").and_then(Value::as_bool).unwrap_or(false) { "streaming" } else { "complete" },
        "blocked": value.get("blocked").cloned().unwrap_or(Value::Null),
        "url": value.get("url").cloned().unwrap_or(json!("")),
        "site": value.get("site").cloned().unwrap_or(json!(""))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_accepts_official_https_hosts_only() {
        assert_eq!(detect("https://chatgpt.com/"), Some(ChatSite::ChatGpt));
        assert_eq!(detect("https://www.chatgpt.com/"), Some(ChatSite::ChatGpt));
        assert_eq!(
            detect("https://chat.openai.com/c/abc"),
            Some(ChatSite::ChatGpt)
        );
        assert_eq!(
            detect("https://gemini.google.com/app"),
            Some(ChatSite::Gemini)
        );
        assert_eq!(
            detect("https://www.gemini.google.com/"),
            Some(ChatSite::Gemini)
        );
        assert_eq!(
            detect("https://www.google.com/search?udm=50"),
            Some(ChatSite::GoogleAi)
        );
        assert_eq!(
            detect("https://google.com/search?q=rna&udm=50&aep=1"),
            Some(ChatSite::GoogleAi)
        );
        assert!(detect("https://www.google.com/search?q=rna").is_none());
        assert!(detect("https://www.google.com/search?q=udm%3D50").is_none());
        assert!(detect("https://example.com/chatgpt").is_none());
        assert!(detect("https://chatgpt.com.evil.com/").is_none());
        assert!(detect("https://evilchatgpt.com/").is_none());
        assert!(detect("https://gemini.google.com.evil.com/").is_none());
        assert!(detect("https://google.com.evil.com/search?udm=50").is_none());
        assert!(detect("https://evil.com/?next=chatgpt.com").is_none());
        assert!(detect("http://chatgpt.com/").is_none());
        assert!(detect("http://gemini.google.com/").is_none());
        assert!(detect("javascript:alert('chatgpt.com')").is_none());
        assert!(detect("not a url chatgpt.com").is_none());
    }
}
