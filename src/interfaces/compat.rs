use axum::http::{HeaderMap, header};
use serde_json::Value;

use crate::{
    application::state::SharedState,
    protocol::ConnectAuth,
    security::auth::{self, AuthFailureReason},
};

pub(crate) fn authorize_gateway_http(
    state: &SharedState,
    headers: &HeaderMap,
) -> Result<(), AuthFailureReason> {
    let auth = auth_from_headers(headers);
    auth::authorize(&state.config().auth_mode, auth.as_ref())
}

pub(crate) fn normalize_segment(value: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;

    for ch in value.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            out.push(lower);
            pending_dash = false;
            continue;
        }

        if lower == '_' || lower == '-' || lower == ':' || lower.is_ascii_whitespace() {
            pending_dash = true;
        }
    }

    out.trim_matches('-').to_owned()
}

pub(crate) fn extract_text_content(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_owned();
    }

    let read_part = |part: &Value| -> Option<String> {
        let obj = part.as_object()?;
        let content_type = obj.get("type").and_then(Value::as_str).unwrap_or_default();
        let text = obj.get("text").and_then(Value::as_str);
        let input_text = obj.get("input_text").and_then(Value::as_str);

        match content_type {
            "text" | "input_text" => text.or(input_text).map(str::to_owned),
            _ => input_text.or(text).map(str::to_owned),
        }
    };

    if let Some(parts) = content.as_array() {
        return parts
            .iter()
            .filter_map(read_part)
            .filter(|value| !value.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
    }

    read_part(content).unwrap_or_default()
}

fn auth_from_headers(headers: &HeaderMap) -> Option<ConnectAuth> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = raw
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    Some(ConnectAuth {
        token: Some(token.to_owned()),
        device_token: None,
        // For HTTP, bearer auth is accepted in both token and password modes.
        password: Some(token.to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{extract_text_content, normalize_segment};

    #[test]
    fn normalize_segment_collapses_separators() {
        assert_eq!(normalize_segment("gpt-4o-mini"), "gpt-4o-mini");
        assert_eq!(normalize_segment("User Name 123"), "user-name-123");
    }

    #[test]
    fn extract_text_content_supports_array_payloads() {
        let content = json!([
            {"type": "input_text", "text": "first line"},
            {"type": "text", "text": "second line"}
        ]);
        assert_eq!(extract_text_content(&content), "first line\nsecond line");
    }
}
