use futures_util::SinkExt;
use reclaw_core::application::config::AuthMode;
use reclaw_core::protocol::PROTOCOL_VERSION;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

use super::support::{connect_frame, connect_gateway, recv_json, rpc_req, spawn_server};

#[tokio::test]
async fn handshake_and_health_round_trip() {
    let server = spawn_server(AuthMode::None).await;
    let mut ws = connect_gateway(server.addr).await;

    ws.send(Message::Text(
        connect_frame(None, 1, PROTOCOL_VERSION, "operator", "reclaw-test", &[])
            .to_string()
            .into(),
    ))
    .await
    .expect("connect frame should send");

    let hello = recv_json(&mut ws).await;
    assert_eq!(hello["type"], "res");
    assert_eq!(hello["id"], "connect-1");
    assert_eq!(hello["ok"], true);
    assert_eq!(hello["payload"]["type"], "hello-ok");
    assert_eq!(hello["payload"]["protocol"], PROTOCOL_VERSION);

    let health = rpc_req(&mut ws, "h-1", "health", None).await;
    assert_eq!(health["type"], "res");
    assert_eq!(health["id"], "h-1");
    assert_eq!(health["ok"], true);
    assert_eq!(health["payload"]["ok"], true);

    server.stop().await;
}

#[tokio::test]
async fn handshake_rejects_protocol_mismatch() {
    let server = spawn_server(AuthMode::None).await;
    let mut ws = connect_gateway(server.addr).await;

    ws.send(Message::Text(
        connect_frame(
            None,
            PROTOCOL_VERSION + 1,
            PROTOCOL_VERSION + 1,
            "operator",
            "reclaw-test",
            &[],
        )
        .to_string()
        .into(),
    ))
    .await
    .expect("connect frame should send");

    let response = recv_json(&mut ws).await;
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "INVALID_REQUEST");
    assert_eq!(response["error"]["message"], "protocol mismatch");

    server.stop().await;
}

#[tokio::test]
async fn token_auth_rejects_missing_token() {
    let server = spawn_server(AuthMode::Token("top-secret".to_owned())).await;
    let mut ws = connect_gateway(server.addr).await;

    ws.send(Message::Text(
        connect_frame(None, 1, PROTOCOL_VERSION, "operator", "reclaw-test", &[])
            .to_string()
            .into(),
    ))
    .await
    .expect("connect frame should send");

    let response = recv_json(&mut ws).await;
    assert_eq!(response["ok"], false);
    assert_eq!(
        response["error"]["message"],
        "unauthorized: missing credentials"
    );

    server.stop().await;
}

#[tokio::test]
async fn unknown_methods_and_validation_errors_are_explicit() {
    let server = spawn_server(AuthMode::None).await;
    let mut ws = connect_gateway(server.addr).await;

    ws.send(Message::Text(
        connect_frame(None, 1, PROTOCOL_VERSION, "operator", "reclaw-test", &[])
            .to_string()
            .into(),
    ))
    .await
    .expect("connect frame should send");
    let _ = recv_json(&mut ws).await;

    let unknown = rpc_req(&mut ws, "u-1", "unknown.method", None).await;
    assert_eq!(unknown["ok"], false);
    assert_eq!(unknown["error"]["code"], "INVALID_REQUEST");

    let wizard = rpc_req(
        &mut ws,
        "u-2",
        "wizard.start",
        Some(json!({ "goal": "Validate runtime wiring" })),
    )
    .await;
    assert_eq!(wizard["ok"], true);

    server.stop().await;
}

#[tokio::test]
async fn method_groups_round_trip() {
    let server = spawn_server(AuthMode::None).await;
    let mut ws = connect_gateway(server.addr).await;

    ws.send(Message::Text(
        connect_frame(None, 1, PROTOCOL_VERSION, "operator", "reclaw-test", &[])
            .to_string()
            .into(),
    ))
    .await
    .expect("connect frame should send");
    let _ = recv_json(&mut ws).await;

    let set_cfg = rpc_req(
        &mut ws,
        "cfg-1",
        "config.set",
        Some(json!({ "config": { "gateway": { "name": "reclaw" } } })),
    )
    .await;
    assert_eq!(set_cfg["ok"], true);

    let patch_cfg = rpc_req(
        &mut ws,
        "cfg-2",
        "config.patch",
        Some(json!({ "patch": { "gateway": { "port": 1234 } } })),
    )
    .await;
    assert_eq!(patch_cfg["ok"], true);

    let get_cfg = rpc_req(&mut ws, "cfg-3", "config.get", Some(json!({}))).await;
    assert_eq!(get_cfg["ok"], true);
    assert_eq!(get_cfg["payload"]["gateway"]["name"], "reclaw");
    assert_eq!(get_cfg["payload"]["gateway"]["port"], 1234);

    let session_patch = rpc_req(
        &mut ws,
        "sess-1",
        "sessions.patch",
        Some(json!({
            "id": "agent:main:main",
            "title": "Main",
            "tags": ["chat"],
            "metadata": {"source": "test"}
        })),
    )
    .await;
    assert_eq!(session_patch["ok"], true);

    let send = rpc_req(
        &mut ws,
        "chat-1",
        "chat.send",
        Some(json!({
            "sessionKey": "agent:main:main",
            "message": "hello",
            "idempotencyKey": "run-chat-1"
        })),
    )
    .await;
    assert_eq!(send["ok"], true);
    assert_eq!(send["payload"]["status"], "completed");

    let history = rpc_req(
        &mut ws,
        "chat-2",
        "chat.history",
        Some(json!({ "sessionKey": "agent:main:main", "limit": 10 })),
    )
    .await;
    assert_eq!(history["ok"], true);
    assert!(
        history["payload"]["messages"]
            .as_array()
            .is_some_and(|messages| messages.len() >= 2)
    );

    let wait = rpc_req(
        &mut ws,
        "agent-1",
        "agent.wait",
        Some(json!({ "runId": "run-chat-1", "timeoutMs": 500 })),
    )
    .await;
    assert_eq!(wait["ok"], true);
    assert_eq!(wait["payload"]["status"], "completed");

    let add_job = rpc_req(
        &mut ws,
        "cron-1",
        "cron.add",
        Some(json!({
            "id": "job-1",
            "name": "Job One",
            "enabled": true,
            "schedule": { "kind": "every", "everyMs": 1000 },
            "payload": { "kind": "systemEvent", "text": "tick" },
            "metadata": {}
        })),
    )
    .await;
    assert_eq!(add_job["ok"], true);

    let run_job = rpc_req(
        &mut ws,
        "cron-2",
        "cron.run",
        Some(json!({ "id": "job-1" })),
    )
    .await;
    assert_eq!(run_job["ok"], true);

    let runs = rpc_req(
        &mut ws,
        "cron-3",
        "cron.runs",
        Some(json!({ "id": "job-1", "limit": 10 })),
    )
    .await;
    assert_eq!(runs["ok"], true);
    assert!(
        runs["payload"]["count"]
            .as_u64()
            .is_some_and(|count| count >= 1)
    );

    let pair_request = rpc_req(
        &mut ws,
        "node-1",
        "node.pair.request",
        Some(json!({
            "nodeId": "node-a",
            "displayName": "Node A",
            "platform": "ios",
            "commands": ["ping"]
        })),
    )
    .await;
    assert_eq!(pair_request["ok"], true);

    let request_id = pair_request["payload"]["request"]["requestId"]
        .as_str()
        .expect("request id should exist")
        .to_owned();

    let approve = rpc_req(
        &mut ws,
        "node-2",
        "node.pair.approve",
        Some(json!({ "requestId": request_id })),
    )
    .await;
    assert_eq!(approve["ok"], true);

    let node_list = rpc_req(&mut ws, "node-3", "node.list", None).await;
    assert_eq!(node_list["ok"], true);

    let rename = rpc_req(
        &mut ws,
        "node-4",
        "node.rename",
        Some(json!({ "nodeId": "node-a", "displayName": "Node Renamed" })),
    )
    .await;
    assert_eq!(rename["ok"], true);

    let invoke = rpc_req(
        &mut ws,
        "node-5",
        "node.invoke",
        Some(json!({
            "nodeId": "node-a",
            "command": "test.command",
            "args": ["a"]
        })),
    )
    .await;
    assert_eq!(invoke["ok"], true);

    let invoke_id = invoke["payload"]["requestId"]
        .as_str()
        .expect("invoke id should exist")
        .to_owned();

    let mut node_ws = connect_gateway(server.addr).await;
    node_ws
        .send(Message::Text(
            connect_frame(None, 1, PROTOCOL_VERSION, "node", "node-a", &[])
                .to_string()
                .into(),
        ))
        .await
        .expect("node connect frame should send");
    let node_hello = recv_json(&mut node_ws).await;
    assert_eq!(node_hello["ok"], true);

    let invoke_result = rpc_req(
        &mut node_ws,
        "node-6",
        "node.invoke.result",
        Some(json!({
            "requestId": invoke_id,
            "status": "completed",
            "payload": { "ok": true }
        })),
    )
    .await;
    assert_eq!(invoke_result["ok"], true);

    let node_event = rpc_req(
        &mut node_ws,
        "node-7",
        "node.event",
        Some(json!({
            "event": "heartbeat",
            "payload": { "ok": true }
        })),
    )
    .await;
    assert_eq!(node_event["ok"], true);

    let remove_job = rpc_req(
        &mut ws,
        "cron-4",
        "cron.remove",
        Some(json!({ "id": "job-1" })),
    )
    .await;
    assert_eq!(remove_job["ok"], true);
    assert_eq!(remove_job["payload"]["removed"], true);

    server.stop().await;
}

#[tokio::test]
async fn operator_cannot_call_node_role_methods() {
    let server = spawn_server(AuthMode::None).await;
    let mut ws = connect_gateway(server.addr).await;

    ws.send(Message::Text(
        connect_frame(
            None,
            1,
            PROTOCOL_VERSION,
            "operator",
            "reclaw-operator",
            &[],
        )
        .to_string()
        .into(),
    ))
    .await
    .expect("connect frame should send");
    let _ = recv_json(&mut ws).await;

    let denied = rpc_req(
        &mut ws,
        "deny-1",
        "node.invoke.result",
        Some(json!({
            "requestId": "missing",
            "status": "completed"
        })),
    )
    .await;
    assert_eq!(denied["ok"], false);
    assert_eq!(denied["error"]["code"], "INVALID_REQUEST");

    server.stop().await;
}

#[tokio::test]
async fn extended_method_groups_round_trip() {
    let server = spawn_server(AuthMode::None).await;
    let mut ws = connect_gateway(server.addr).await;

    ws.send(Message::Text(
        connect_frame(None, 1, PROTOCOL_VERSION, "operator", "reclaw-ext", &[])
            .to_string()
            .into(),
    ))
    .await
    .expect("connect frame should send");
    let hello = recv_json(&mut ws).await;
    assert_eq!(hello["ok"], true);

    let doctor = rpc_req(&mut ws, "ext-1", "doctor.memory.status", Some(json!({}))).await;
    assert_eq!(doctor["ok"], true);

    let channels = rpc_req(&mut ws, "ext-2", "channels.status", Some(json!({}))).await;
    assert_eq!(channels["ok"], true);

    let logout = rpc_req(
        &mut ws,
        "ext-3",
        "channels.logout",
        Some(json!({ "channel": "webchat" })),
    )
    .await;
    assert_eq!(logout["ok"], true);

    let logs = rpc_req(&mut ws, "ext-4", "logs.tail", Some(json!({ "limit": 10 }))).await;
    assert_eq!(logs["ok"], true);
    assert!(
        logs["payload"]["count"]
            .as_u64()
            .is_some_and(|count| count >= 1)
    );

    let talk_mode = rpc_req(
        &mut ws,
        "ext-5",
        "talk.mode",
        Some(json!({ "mode": "focus" })),
    )
    .await;
    assert_eq!(talk_mode["ok"], true);

    let talk_config = rpc_req(&mut ws, "ext-6", "talk.config", Some(json!({}))).await;
    assert_eq!(talk_config["ok"], true);
    assert_eq!(talk_config["payload"]["mode"], "focus");

    let models = rpc_req(&mut ws, "ext-7", "models.list", Some(json!({}))).await;
    assert_eq!(models["ok"], true);

    let catalog = rpc_req(&mut ws, "ext-8", "tools.catalog", Some(json!({}))).await;
    assert_eq!(catalog["ok"], true);

    let tts_enable = rpc_req(&mut ws, "ext-9", "tts.enable", Some(json!({}))).await;
    assert_eq!(tts_enable["ok"], true);

    let tts_set_provider = rpc_req(
        &mut ws,
        "ext-10",
        "tts.setProvider",
        Some(json!({ "provider": "mock" })),
    )
    .await;
    assert_eq!(tts_set_provider["ok"], true);

    let tts_convert = rpc_req(
        &mut ws,
        "ext-11",
        "tts.convert",
        Some(json!({ "text": "hello from tts" })),
    )
    .await;
    assert_eq!(tts_convert["ok"], true);

    let voicewake_set = rpc_req(
        &mut ws,
        "ext-12",
        "voicewake.set",
        Some(json!({ "enabled": true, "phrase": "hey reclaw core" })),
    )
    .await;
    assert_eq!(voicewake_set["ok"], true);

    let voicewake_get = rpc_req(&mut ws, "ext-13", "voicewake.get", Some(json!({}))).await;
    assert_eq!(voicewake_get["ok"], true);
    assert_eq!(voicewake_get["payload"]["enabled"], true);

    let wizard_start = rpc_req(
        &mut ws,
        "ext-14",
        "wizard.start",
        Some(json!({ "goal": "bootstrap server" })),
    )
    .await;
    assert_eq!(wizard_start["ok"], true);
    let wizard_id = wizard_start["payload"]["id"]
        .as_str()
        .expect("wizard id should exist")
        .to_owned();

    let wizard_next = rpc_req(
        &mut ws,
        "ext-15",
        "wizard.next",
        Some(json!({ "id": wizard_id, "input": "continue" })),
    )
    .await;
    assert_eq!(wizard_next["ok"], true);

    let system_presence = rpc_req(&mut ws, "ext-16", "system-presence", Some(json!({}))).await;
    assert_eq!(system_presence["ok"], true);

    let wake = rpc_req(
        &mut ws,
        "ext-17",
        "wake",
        Some(json!({ "reason": "integration-test" })),
    )
    .await;
    assert_eq!(wake["ok"], true);

    let last_heartbeat = rpc_req(&mut ws, "ext-18", "last-heartbeat", Some(json!({}))).await;
    assert_eq!(last_heartbeat["ok"], true);

    let set_heartbeats = rpc_req(
        &mut ws,
        "ext-19",
        "set-heartbeats",
        Some(json!({ "heartbeats": { "gateway": true } })),
    )
    .await;
    assert_eq!(set_heartbeats["ok"], true);

    let system_event = rpc_req(
        &mut ws,
        "ext-20",
        "system-event",
        Some(json!({ "event": "integration", "payload": { "ok": true } })),
    )
    .await;
    assert_eq!(system_event["ok"], true);

    let send = rpc_req(
        &mut ws,
        "ext-21",
        "send",
        Some(json!({ "sessionKey": "agent:main:main", "message": "outbound" })),
    )
    .await;
    assert_eq!(send["ok"], true);

    let usage_status = rpc_req(&mut ws, "ext-22", "usage.status", Some(json!({}))).await;
    assert_eq!(usage_status["ok"], true);

    let usage_cost = rpc_req(
        &mut ws,
        "ext-23",
        "usage.cost",
        Some(json!({ "periodDays": 7 })),
    )
    .await;
    assert_eq!(usage_cost["ok"], true);

    let agents_before = rpc_req(&mut ws, "ext-24", "agents.list", Some(json!({}))).await;
    assert_eq!(agents_before["ok"], true);

    let create_agent = rpc_req(
        &mut ws,
        "ext-25",
        "agents.create",
        Some(json!({ "name": "Build Bot" })),
    )
    .await;
    assert_eq!(create_agent["ok"], true);
    let agent_id = create_agent["payload"]["agentId"]
        .as_str()
        .expect("agent id should exist")
        .to_owned();

    let set_file = rpc_req(
        &mut ws,
        "ext-26",
        "agents.files.set",
        Some(json!({
            "agentId": agent_id,
            "name": "IDENTITY.md",
            "content": "# Identity\n- Name: Build Bot\n"
        })),
    )
    .await;
    assert_eq!(set_file["ok"], true);

    let get_file = rpc_req(
        &mut ws,
        "ext-27",
        "agents.files.get",
        Some(json!({ "agentId": agent_id, "name": "IDENTITY.md" })),
    )
    .await;
    assert_eq!(get_file["ok"], true);

    let update_agent = rpc_req(
        &mut ws,
        "ext-28",
        "agents.update",
        Some(json!({ "agentId": agent_id, "model": "gpt-5" })),
    )
    .await;
    assert_eq!(update_agent["ok"], true);

    let delete_agent = rpc_req(
        &mut ws,
        "ext-29",
        "agents.delete",
        Some(json!({ "agentId": agent_id, "deleteFiles": true })),
    )
    .await;
    assert_eq!(delete_agent["ok"], true);

    let skills_install = rpc_req(
        &mut ws,
        "ext-30",
        "skills.install",
        Some(json!({ "name": "node-tools", "installId": "demo/node-tools" })),
    )
    .await;
    assert_eq!(skills_install["ok"], true);

    let skills_update = rpc_req(
        &mut ws,
        "ext-31",
        "skills.update",
        Some(json!({
            "skillKey": "node-tools",
            "enabled": true,
            "env": { "API_KEY": "secret" }
        })),
    )
    .await;
    assert_eq!(skills_update["ok"], true);

    let skills_status = rpc_req(&mut ws, "ext-32", "skills.status", Some(json!({}))).await;
    assert_eq!(skills_status["ok"], true);

    let mut node_ws = connect_gateway(server.addr).await;
    node_ws
        .send(Message::Text(
            connect_frame(None, 1, PROTOCOL_VERSION, "node", "skills-node", &[])
                .to_string()
                .into(),
        ))
        .await
        .expect("node connect frame should send");
    let node_hello = recv_json(&mut node_ws).await;
    assert_eq!(node_hello["ok"], true);

    let skills_bins = rpc_req(&mut node_ws, "ext-33", "skills.bins", Some(json!({}))).await;
    assert_eq!(skills_bins["ok"], true);

    let approvals_get = rpc_req(&mut ws, "ext-34", "exec.approvals.get", Some(json!({}))).await;
    assert_eq!(approvals_get["ok"], true);

    let approvals_set = rpc_req(
        &mut ws,
        "ext-35",
        "exec.approvals.set",
        Some(json!({ "file": { "allow": ["ls"] } })),
    )
    .await;
    assert_eq!(approvals_set["ok"], true);

    let approvals_get_after =
        rpc_req(&mut ws, "ext-36", "exec.approvals.get", Some(json!({}))).await;
    assert_eq!(approvals_get_after["ok"], true);
    let approvals_hash = approvals_get_after["payload"]["hash"]
        .as_str()
        .expect("approvals hash should exist")
        .to_owned();

    let approvals_set_with_hash = rpc_req(
        &mut ws,
        "ext-37",
        "exec.approvals.set",
        Some(json!({
            "baseHash": approvals_hash,
            "file": { "allow": ["ls", "cat"] }
        })),
    )
    .await;
    assert_eq!(approvals_set_with_hash["ok"], true);

    let approvals_node_get = rpc_req(
        &mut ws,
        "ext-38",
        "exec.approvals.node.get",
        Some(json!({ "nodeId": "node-z" })),
    )
    .await;
    assert_eq!(approvals_node_get["ok"], true);

    let approvals_node_set = rpc_req(
        &mut ws,
        "ext-39",
        "exec.approvals.node.set",
        Some(json!({
            "nodeId": "node-z",
            "file": { "allow": ["echo"] }
        })),
    )
    .await;
    assert_eq!(approvals_node_set["ok"], true);

    let approval_request = rpc_req(
        &mut ws,
        "ext-40",
        "exec.approval.request",
        Some(json!({
            "command": "ls -la",
            "twoPhase": true,
            "timeoutMs": 5000
        })),
    )
    .await;
    assert_eq!(approval_request["ok"], true);
    let approval_id = approval_request["payload"]["id"]
        .as_str()
        .expect("approval id should exist")
        .to_owned();

    let approval_resolve = rpc_req(
        &mut ws,
        "ext-41",
        "exec.approval.resolve",
        Some(json!({ "id": approval_id, "decision": "allow-once" })),
    )
    .await;
    assert_eq!(approval_resolve["ok"], true);

    let approval_wait = rpc_req(
        &mut ws,
        "ext-42",
        "exec.approval.waitDecision",
        Some(json!({ "id": approval_id, "timeoutMs": 2000 })),
    )
    .await;
    assert_eq!(approval_wait["ok"], true);
    assert_eq!(approval_wait["payload"]["decision"], "allow-once");

    let update = rpc_req(
        &mut ws,
        "ext-43",
        "update.run",
        Some(json!({ "mode": "check" })),
    )
    .await;
    assert_eq!(update["ok"], true);

    let pair_request = rpc_req(
        &mut ws,
        "ext-44",
        "node.pair.request",
        Some(json!({
            "nodeId": "device-node-1",
            "displayName": "Device Node",
            "platform": "ios",
            "commands": ["status"]
        })),
    )
    .await;
    assert_eq!(pair_request["ok"], true);
    let pair_request_id = pair_request["payload"]["request"]["requestId"]
        .as_str()
        .expect("pair request id should exist")
        .to_owned();

    let device_approve = rpc_req(
        &mut ws,
        "ext-45",
        "device.pair.approve",
        Some(json!({ "requestId": pair_request_id })),
    )
    .await;
    assert_eq!(device_approve["ok"], true);

    let device_list = rpc_req(&mut ws, "ext-46", "device.pair.list", Some(json!({}))).await;
    assert_eq!(device_list["ok"], true);

    let token_rotate = rpc_req(
        &mut ws,
        "ext-47",
        "device.token.rotate",
        Some(json!({
            "deviceId": "device-node-1",
            "role": "operator",
            "scopes": ["operator.read"]
        })),
    )
    .await;
    assert_eq!(token_rotate["ok"], true);

    let token_revoke = rpc_req(
        &mut ws,
        "ext-48",
        "device.token.revoke",
        Some(json!({
            "deviceId": "device-node-1",
            "role": "operator"
        })),
    )
    .await;
    assert_eq!(token_revoke["ok"], true);

    let device_remove = rpc_req(
        &mut ws,
        "ext-49",
        "device.pair.remove",
        Some(json!({ "deviceId": "device-node-1" })),
    )
    .await;
    assert_eq!(device_remove["ok"], true);

    let browser = rpc_req(
        &mut ws,
        "ext-50",
        "browser.request",
        Some(json!({ "action": "open" })),
    )
    .await;
    assert_eq!(browser["ok"], false);
    assert_eq!(browser["error"]["code"], "UNAVAILABLE");

    server.stop().await;
}
