use serde_json::json;

use crate::{
    application::state::SharedState,
    domain::error::DomainError,
    protocol::{
        ERROR_INVALID_REQUEST, ERROR_NOT_PAIRED, ERROR_UNAVAILABLE, ErrorShape, RequestFrame,
        ResponseFrame, response_error, response_ok,
    },
    rpc::{SessionContext, methods, policy},
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

    if let Err(error) = policy::authorize_session(session, &request.method) {
        return response_error(request.id.clone(), error);
    }

    if policy::is_control_plane_write_method(&request.method) {
        let key = format!("{}:{}", session.client_id, request.method);
        let decision = state
            .control_plane_rate_limiter()
            .record_failure(&key)
            .await;
        if !decision.allowed {
            let error = ErrorShape::new(
                ERROR_UNAVAILABLE,
                format!(
                    "rate limit exceeded for {}; retry after {}s",
                    request.method,
                    decision.retry_after_ms.div_ceil(1_000)
                ),
            )
            .with_retry(decision.retry_after_ms)
            .with_details(json!({
                "method": request.method,
                "limit": "3 per 60s"
            }));
            return response_error(request.id.clone(), error);
        }
    }

    let _ = state
        .append_gateway_log(
            "info",
            &format!("rpc request method={}", request.method),
            Some(&request.method),
            Some(&session.conn_id),
        )
        .await;

    let result = match request.method.as_str() {
        "health" => Ok(methods::health::handle(state, request.params.as_ref()).await),
        "doctor.memory.status" => {
            methods::doctor::handle_memory_status(state, request.params.as_ref()).await
        }
        "logs.tail" => methods::logs::handle_tail(state, request.params.as_ref()).await,
        "channels.status" => methods::channels::handle_status(state, request.params.as_ref()).await,
        "channels.logout" => methods::channels::handle_logout(state, request.params.as_ref()).await,
        "status" => Ok(methods::status::handle(state, session).await),
        "usage.status" => methods::usage::handle_status(state, request.params.as_ref()).await,
        "usage.cost" => methods::usage::handle_cost(state, request.params.as_ref()).await,
        "tts.status" => methods::tts::handle_status(state, request.params.as_ref()).await,
        "tts.providers" => methods::tts::handle_providers(state, request.params.as_ref()).await,
        "tts.enable" => methods::tts::handle_enable(state, request.params.as_ref()).await,
        "tts.disable" => methods::tts::handle_disable(state, request.params.as_ref()).await,
        "tts.convert" => methods::tts::handle_convert(state, request.params.as_ref()).await,
        "tts.setProvider" => {
            methods::tts::handle_set_provider(state, request.params.as_ref()).await
        }
        "config.get" => methods::config::handle_get(state, request.params.as_ref()).await,
        "config.set" => methods::config::handle_set(state, request.params.as_ref()).await,
        "config.apply" => methods::config::handle_apply(state, request.params.as_ref()).await,
        "config.patch" => methods::config::handle_patch(state, request.params.as_ref()).await,
        "config.schema" => Ok(methods::config::handle_schema()),
        "exec.approvals.get" => {
            methods::approvals::handle_exec_approvals_get(state, request.params.as_ref()).await
        }
        "exec.approvals.set" => {
            methods::approvals::handle_exec_approvals_set(state, request.params.as_ref()).await
        }
        "exec.approvals.node.get" => {
            methods::approvals::handle_exec_approvals_node_get(state, request.params.as_ref()).await
        }
        "exec.approvals.node.set" => {
            methods::approvals::handle_exec_approvals_node_set(state, request.params.as_ref()).await
        }
        "exec.approval.request" => {
            methods::approvals::handle_exec_approval_request(
                state,
                session,
                request.params.as_ref(),
            )
            .await
        }
        "exec.approval.waitDecision" => {
            methods::approvals::handle_exec_approval_wait_decision(state, request.params.as_ref())
                .await
        }
        "exec.approval.resolve" => {
            methods::approvals::handle_exec_approval_resolve(
                state,
                session,
                request.params.as_ref(),
            )
            .await
        }
        "wizard.start" => methods::wizard::handle_start(state, request.params.as_ref()).await,
        "wizard.next" => methods::wizard::handle_next(state, request.params.as_ref()).await,
        "wizard.cancel" => methods::wizard::handle_cancel(state, request.params.as_ref()).await,
        "wizard.status" => methods::wizard::handle_status(state, request.params.as_ref()).await,
        "talk.config" => methods::talk::handle_config(state, request.params.as_ref()).await,
        "talk.mode" => methods::talk::handle_mode(state, request.params.as_ref()).await,
        "models.list" => methods::models::handle_list(state, request.params.as_ref()).await,
        "tools.catalog" => methods::tools::handle_catalog(state, request.params.as_ref()),
        "agents.list" => methods::agents::handle_list(state, request.params.as_ref()).await,
        "agents.create" => methods::agents::handle_create(state, request.params.as_ref()).await,
        "agents.update" => methods::agents::handle_update(state, request.params.as_ref()).await,
        "agents.delete" => methods::agents::handle_delete(state, request.params.as_ref()).await,
        "agents.files.list" => {
            methods::agents::handle_files_list(state, request.params.as_ref()).await
        }
        "agents.files.get" => {
            methods::agents::handle_files_get(state, request.params.as_ref()).await
        }
        "agents.files.set" => {
            methods::agents::handle_files_set(state, request.params.as_ref()).await
        }
        "skills.status" => methods::skills::handle_status(state, request.params.as_ref()).await,
        "skills.bins" => methods::skills::handle_bins(state, request.params.as_ref()).await,
        "skills.install" => methods::skills::handle_install(state, request.params.as_ref()).await,
        "skills.update" => methods::skills::handle_update(state, request.params.as_ref()).await,
        "update.run" => methods::update::handle_run(state, session, request.params.as_ref()).await,
        "voicewake.get" => methods::voicewake::handle_get(state, request.params.as_ref()).await,
        "voicewake.set" => methods::voicewake::handle_set(state, request.params.as_ref()).await,
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
        "last-heartbeat" => {
            methods::system::handle_last_heartbeat(state, request.params.as_ref()).await
        }
        "set-heartbeats" => {
            methods::system::handle_set_heartbeats(state, request.params.as_ref()).await
        }
        "wake" => methods::system::handle_wake(state, session, request.params.as_ref()).await,
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
        "device.pair.list" => {
            methods::device::handle_pair_list(state, request.params.as_ref()).await
        }
        "device.pair.approve" => {
            methods::device::handle_pair_approve(state, request.params.as_ref()).await
        }
        "device.pair.reject" => {
            methods::device::handle_pair_reject(state, request.params.as_ref()).await
        }
        "device.pair.remove" => {
            methods::device::handle_pair_remove(state, request.params.as_ref()).await
        }
        "device.token.rotate" => {
            methods::device::handle_token_rotate(state, request.params.as_ref()).await
        }
        "device.token.revoke" => {
            methods::device::handle_token_revoke(state, request.params.as_ref()).await
        }
        "node.rename" => methods::nodes::handle_rename(state, request.params.as_ref()).await,
        "node.list" => methods::nodes::handle_list(state, request.params.as_ref()).await,
        "node.describe" => methods::nodes::handle_describe(state, request.params.as_ref()).await,
        "node.invoke" => methods::nodes::handle_invoke(state, request.params.as_ref()).await,
        "node.invoke.result" => {
            methods::nodes::handle_invoke_result(state, request.params.as_ref()).await
        }
        "node.event" => methods::nodes::handle_event(state, session, request.params.as_ref()).await,
        "cron.list" => methods::cron::handle_list(state, request.params.as_ref()).await,
        "cron.status" => methods::cron::handle_status(state, request.params.as_ref()).await,
        "cron.add" => methods::cron::handle_add(state, request.params.as_ref()).await,
        "cron.update" => methods::cron::handle_update(state, request.params.as_ref()).await,
        "cron.remove" => methods::cron::handle_remove(state, request.params.as_ref()).await,
        "cron.run" => methods::cron::handle_run(state, request.params.as_ref()).await,
        "cron.runs" => methods::cron::handle_runs(state, request.params.as_ref()).await,
        "system-presence" => {
            methods::system::handle_system_presence(state, request.params.as_ref()).await
        }
        "system-event" => {
            methods::system::handle_system_event(state, session, request.params.as_ref()).await
        }
        "send" => methods::send::handle_send(state, session, request.params.as_ref()).await,
        "agent" => methods::agent::handle_agent(state, session, request.params.as_ref()).await,
        "agent.identity.get" => {
            methods::agent::handle_agent_identity(state, request.params.as_ref()).await
        }
        "agent.wait" => methods::agent::handle_agent_wait(state, request.params.as_ref()).await,
        "browser.request" => methods::browser::handle_request(request.params.as_ref()).await,
        "chat.history" => methods::chat::handle_history(state, request.params.as_ref()).await,
        "chat.abort" => methods::chat::handle_abort(state, request.params.as_ref()).await,
        "chat.send" => methods::chat::handle_send(state, session, request.params.as_ref()).await,
        _ => Err(ErrorShape::new(
            ERROR_INVALID_REQUEST,
            format!("unknown method: {}", request.method),
        )),
    };

    match result {
        Ok(payload) => {
            let _ = state
                .append_gateway_log(
                    "info",
                    &format!("rpc success method={}", request.method),
                    Some(&request.method),
                    Some(&session.conn_id),
                )
                .await;
            response_ok(request.id.clone(), payload)
        }
        Err(error) => {
            let _ = state
                .append_gateway_log(
                    "warn",
                    &format!("rpc error method={} code={}", request.method, error.code),
                    Some(&request.method),
                    Some(&session.conn_id),
                )
                .await;
            response_error(request.id.clone(), error)
        }
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
