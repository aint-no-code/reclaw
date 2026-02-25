mod errors;
mod frames;

pub use errors::{
    ERROR_AGENT_TIMEOUT, ERROR_INVALID_REQUEST, ERROR_NOT_LINKED, ERROR_NOT_PAIRED,
    ERROR_UNAVAILABLE, ErrorShape,
};
pub use frames::{
    ConnectAuth, ConnectClient, ConnectParams, GatewayPolicy, HelloFeatures, HelloOk, HelloServer,
    PresenceEntry, RequestFrame, ResponseFrame, Snapshot, StateVersion,
};

use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 3;

pub fn parse_request_frame(text: &str) -> Result<RequestFrame, ErrorShape> {
    let request = serde_json::from_str::<RequestFrame>(text).map_err(|error| {
        ErrorShape::new(
            ERROR_INVALID_REQUEST,
            format!("invalid request frame: {error}"),
        )
    })?;

    if request.frame_type != "req" {
        return Err(ErrorShape::new(
            ERROR_INVALID_REQUEST,
            "invalid request frame: expected type=req",
        ));
    }
    if request.id.trim().is_empty() {
        return Err(ErrorShape::new(
            ERROR_INVALID_REQUEST,
            "invalid request frame: missing id",
        ));
    }
    if request.method.trim().is_empty() {
        return Err(ErrorShape::new(
            ERROR_INVALID_REQUEST,
            "invalid request frame: missing method",
        ));
    }

    Ok(request)
}

#[must_use]
pub fn response_ok(id: impl Into<String>, payload: Value) -> ResponseFrame {
    ResponseFrame {
        frame_type: "res",
        id: id.into(),
        ok: true,
        payload: Some(payload),
        error: None,
    }
}

#[must_use]
pub fn response_error(id: impl Into<String>, error: ErrorShape) -> ResponseFrame {
    ResponseFrame {
        frame_type: "res",
        id: id.into(),
        ok: false,
        payload: None,
        error: Some(error),
    }
}
