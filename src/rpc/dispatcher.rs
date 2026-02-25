use serde_json::json;

use crate::{
    application::state::SharedState,
    domain::error::DomainError,
    protocol::{
        ERROR_INVALID_REQUEST, ERROR_NOT_PAIRED, ERROR_UNAVAILABLE, ErrorShape, RequestFrame,
        ResponseFrame, response_error, response_ok,
    },
    rpc::{SessionContext, methods},
};

pub async fn dispatch_request(
    state: &SharedState,
    session: &SessionContext,
    request: &RequestFrame,
) -> ResponseFrame {
    if request.method == "connect" {
        return response_error(
            request.id.clone(),
            ErrorShape::new(
                ERROR_INVALID_REQUEST,
                "connect can only be used as the first handshake request",
            ),
        );
    }

    let result = match request.method.as_str() {
        "health" => Ok(methods::health::handle(state, request.params.as_ref()).await),
        "status" => Ok(methods::status::handle(state, session).await),
        "config.get" => methods::config::handle_get(state, request.params.as_ref()).await,
        "config.set" => methods::config::handle_set(state, request.params.as_ref()).await,
        "config.apply" => methods::config::handle_apply(state, request.params.as_ref()).await,
        "config.patch" => methods::config::handle_patch(state, request.params.as_ref()).await,
        "config.schema" => Ok(methods::config::handle_schema()),
        "sessions.list" => methods::sessions::handle_list(state, request.params.as_ref()).await,
        "sessions.preview" => {
            methods::sessions::handle_preview(state, request.params.as_ref()).await
        }
        "sessions.patch" => methods::sessions::handle_patch(state, request.params.as_ref()).await,
        "sessions.reset" => methods::sessions::handle_reset(state).await,
        "sessions.delete" => methods::sessions::handle_delete(state, request.params.as_ref()).await,
        "sessions.compact" => {
            methods::sessions::handle_compact(state, request.params.as_ref()).await
        }
        "agent" => methods::agent::handle_agent(state, session, request.params.as_ref()).await,
        "agent.wait" => methods::agent::handle_agent_wait(state, request.params.as_ref()).await,
        "agent.identity.get" => {
            methods::agent::handle_agent_identity(state, request.params.as_ref()).await
        }
        "chat.send" => methods::chat::handle_send(state, session, request.params.as_ref()).await,
        "chat.history" => methods::chat::handle_history(state, request.params.as_ref()).await,
        "chat.abort" => methods::chat::handle_abort(state, request.params.as_ref()).await,
        "cron.list" => methods::cron::handle_list(state, request.params.as_ref()).await,
        "cron.status" => methods::cron::handle_status(state, request.params.as_ref()).await,
        "cron.add" => methods::cron::handle_add(state, request.params.as_ref()).await,
        "cron.update" => methods::cron::handle_update(state, request.params.as_ref()).await,
        "cron.remove" => methods::cron::handle_remove(state, request.params.as_ref()).await,
        "cron.run" => methods::cron::handle_run(state, request.params.as_ref()).await,
        "cron.runs" => methods::cron::handle_runs(state, request.params.as_ref()).await,
        "node.pair.request" => {
            methods::nodes::handle_pair_request(state, request.params.as_ref()).await
        }
        "node.pair.list" => methods::nodes::handle_pair_list(state, request.params.as_ref()).await,
        "node.pair.approve" => {
            methods::nodes::handle_pair_approve(state, request.params.as_ref()).await
        }
        "node.pair.reject" => {
            methods::nodes::handle_pair_reject(state, request.params.as_ref()).await
        }
        "node.pair.verify" => {
            methods::nodes::handle_pair_verify(state, request.params.as_ref()).await
        }
        "node.rename" => methods::nodes::handle_rename(state, request.params.as_ref()).await,
        "node.list" => methods::nodes::handle_list(state, request.params.as_ref()).await,
        "node.describe" => methods::nodes::handle_describe(state, request.params.as_ref()).await,
        "node.invoke" => methods::nodes::handle_invoke(state, request.params.as_ref()).await,
        "node.invoke.result" => {
            methods::nodes::handle_invoke_result(state, request.params.as_ref()).await
        }
        "node.event" => methods::nodes::handle_event(state, session, request.params.as_ref()).await,
        _ => {
            if methods::is_known_method(&request.method) {
                let error = ErrorShape::new(
                    ERROR_UNAVAILABLE,
                    format!(
                        "method \"{}\" is recognized but not implemented yet",
                        request.method
                    ),
                )
                .with_details(json!({
                    "method": request.method,
                    "implemented": methods::implemented_methods()
                }))
                .with_retry(1_000);
                Err(error)
            } else {
                Err(ErrorShape::new(
                    ERROR_INVALID_REQUEST,
                    format!("unknown method: {}", request.method),
                ))
            }
        }
    };

    match result {
        Ok(payload) => response_ok(request.id.clone(), payload),
        Err(error) => response_error(request.id.clone(), error),
    }
}

#[must_use]
pub fn map_domain_error(error: DomainError) -> ErrorShape {
    match error {
        DomainError::InvalidRequest(message) => ErrorShape::new(ERROR_INVALID_REQUEST, message),
        DomainError::NotFound(message) => ErrorShape::new(ERROR_INVALID_REQUEST, message),
        DomainError::NotPaired(message) => ErrorShape::new(ERROR_NOT_PAIRED, message),
        DomainError::Unauthorized(message) => ErrorShape::new(ERROR_UNAVAILABLE, message),
        DomainError::Unavailable(message) => ErrorShape::new(ERROR_UNAVAILABLE, message),
        DomainError::Storage(message) => ErrorShape::new(ERROR_UNAVAILABLE, message),
    }
}
