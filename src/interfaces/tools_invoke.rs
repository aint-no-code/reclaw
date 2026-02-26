use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    application::state::SharedState,
    protocol::{ERROR_INVALID_REQUEST, ERROR_UNAVAILABLE, RequestFrame},
    rpc::{SessionContext, dispatcher::dispatch_request, policy},
    security::auth,
};

use super::compat::authorize_gateway_http;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolsInvokeRequest {
    tool: String,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    args: Option<Value>,
    #[serde(default)]
    session_key: Option<String>,
}

pub async fn invoke_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    payload: Result<Json<Value>, JsonRejection>,
) -> Response {
    if let Err(reason) = authorize_gateway_http(&state, &headers) {
        let message = auth::auth_failure_error(reason).message;
        return invoke_error(
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            ERROR_INVALID_REQUEST,
            &message,
        );
    }

    let Json(raw_payload) = match payload {
        Ok(payload) => payload,
        Err(_) => {
            return invoke_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                ERROR_INVALID_REQUEST,
                "invalid JSON body",
            );
        }
    };

    let request: ToolsInvokeRequest = match serde_json::from_value(raw_payload) {
        Ok(request) => request,
        Err(error) => {
            return invoke_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                ERROR_INVALID_REQUEST,
                &format!("invalid tools.invoke body: {error}"),
            );
        }
    };

    let tool = request.tool.trim();
    if tool.is_empty() {
        return invoke_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            ERROR_INVALID_REQUEST,
            "tools.invoke requires body.tool",
        );
    }

    if tool != "gateway.request" {
        return invoke_error(
            StatusCode::NOT_FOUND,
            "not_found",
            ERROR_INVALID_REQUEST,
            &format!("tool not available: {tool}"),
        );
    }

    let args = request
        .args
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    let args_obj = match args.as_object() {
        Some(obj) => obj,
        None => {
            return invoke_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                ERROR_INVALID_REQUEST,
                "tools.invoke gateway.request requires object args",
            );
        }
    };

    let action_method = request
        .action
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(method) = args_obj
        .get("method")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(action_method)
    else {
        return invoke_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            ERROR_INVALID_REQUEST,
            "tools.invoke gateway.request requires args.method or action",
        );
    };

    let params = args_obj.get("params").cloned();
    let session_key = request
        .session_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("agent:main:main");

    let rpc_request = RequestFrame {
        frame_type: "req".to_owned(),
        id: format!("tools-invoke-{}", uuid::Uuid::new_v4()),
        method: method.to_owned(),
        params,
    };
    let session = SessionContext {
        conn_id: format!("http-tools-invoke-{}", uuid::Uuid::new_v4()),
        role: "operator".to_owned(),
        scopes: policy::default_operator_scopes(),
        client_id: format!("tools-invoke:{session_key}"),
        client_mode: "tools-invoke-http".to_owned(),
    };

    let response = dispatch_request(&state, &session, &rpc_request).await;
    if response.ok {
        return (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "result": response.payload.unwrap_or(Value::Null),
            })),
        )
            .into_response();
    }

    let Some(error) = response.error else {
        return invoke_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "api_error",
            ERROR_UNAVAILABLE,
            "rpc request failed without error payload",
        );
    };
    let status = match error.code.as_str() {
        ERROR_INVALID_REQUEST => StatusCode::BAD_REQUEST,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    let error_type = if status == StatusCode::BAD_REQUEST {
        "invalid_request"
    } else {
        "api_error"
    };

    (
        status,
        Json(json!({
            "ok": false,
            "error": {
                "type": error_type,
                "code": error.code,
                "message": error.message,
                "details": error.details,
                "retryable": error.retryable,
                "retryAfterMs": error.retry_after_ms,
            }
        })),
    )
        .into_response()
}

fn invoke_error(status: StatusCode, error_type: &str, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({
            "ok": false,
            "error": {
                "type": error_type,
                "code": code,
                "message": message,
            }
        })),
    )
        .into_response()
}
