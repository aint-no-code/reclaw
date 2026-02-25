use serde_json::{Value, json};

use crate::rpc::methods::parse_required_params;

pub async fn handle_request(_params: Option<&Value>) -> Result<Value, crate::protocol::ErrorShape> {
    let request: serde_json::Map<String, Value> =
        parse_required_params("browser.request", _params)?;

    Err(crate::protocol::ErrorShape::new(
        crate::protocol::ERROR_UNAVAILABLE,
        "browser bridge is unavailable in reclaw-core runtime",
    )
    .with_details(json!({
        "request": request,
        "hint": "route browser traffic through node.invoke on a browser-capable node",
    })))
}
