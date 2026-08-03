use super::*;

const SLACK_RECONNECT_INITIAL_BACKOFF: Duration = Duration::from_secs(3);
const SLACK_RECONNECT_MAX_BACKOFF: Duration = Duration::from_secs(30);

impl SovereignGatewayService {
    pub(super) async fn slack_access(
        &self,
        persistence: PersistenceEngine,
    ) -> Result<crate::native_app_ports::SlackGatewayCredential, String> {
        let connector_id = load_slack_settings(persistence.clone()).await?.connector_id;
        let identity = self.identity.clone();
        tauri::async_runtime::spawn_blocking(move || {
            crate::native_app_ports::resolve_slack_credential(
                &persistence,
                &connector_id,
                &identity,
            )
        })
        .await
        .map_err(|error| error.to_string())?
    }
}

#[derive(Clone, Debug)]
pub(super) struct SlackChannelConfig {
    pub connector_id: String,
    pub owner_id: String,
    pub allowlist_channels: HashSet<String>,
}

pub(super) fn slack_config_from_channel(
    config: &ChannelConfigRecord,
) -> Result<SlackChannelConfig, String> {
    let parsed = serde_json::from_str::<Value>(&config.credentials_json)
        .map_err(|_| "slack_channel_settings_invalid".to_string())?;
    let connector_id = credential_string(&parsed, &["connectorId", "connector_id"])
        .ok_or_else(|| "slack_connector_required".to_string())?;
    let owner_id = clean_optional_text(config.owner_id.as_deref())
        .ok_or_else(|| "slack_owner_required".to_string())?;
    let allowlist_channels =
        credential_string_list(&parsed, &["allowlistChannels", "allowlist_channels"]);
    if allowlist_channels.is_empty() {
        return Err("slack_channel_allowlist_required".to_string());
    }
    Ok(SlackChannelConfig {
        connector_id,
        owner_id,
        allowlist_channels,
    })
}

pub(super) fn spawn_slack_worker(
    inner_ref: Arc<Mutex<GatewayServiceInner>>,
    incoming_sender: mpsc::Sender<GatewayIncomingMessage>,
    persistence: PersistenceEngine,
    identity: SovereignIdentity,
    settings: SlackChannelConfig,
    completion_flag: Arc<AtomicBool>,
) -> JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let _completion_guard = GatewayWorkerCompletionGuard(completion_flag);
        let mut backoff = SLACK_RECONNECT_INITIAL_BACKOFF;
        loop {
            set_worker_status(
                &inner_ref,
                "slack",
                "connecting",
                inactive_connection_state("slack"),
                Some("connecting"),
            );
            let result = run_slack_socket_session(
                Arc::clone(&inner_ref),
                incoming_sender.clone(),
                persistence.clone(),
                identity.clone(),
                &settings,
            )
            .await;
            if let Err(error) = result {
                set_worker_status(
                    &inner_ref,
                    "slack",
                    "running",
                    "error",
                    Some("reconnecting"),
                );
                eprintln!(
                    "SOVEREIGN_GATEWAY_SLACK_CONNECTION_DROPPED retry_seconds={} error={}",
                    backoff.as_secs(),
                    compact_log_text(&error, 180)
                );
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(SLACK_RECONNECT_MAX_BACKOFF);
        }
    })
}

async fn run_slack_socket_session(
    inner_ref: Arc<Mutex<GatewayServiceInner>>,
    incoming_sender: mpsc::Sender<GatewayIncomingMessage>,
    persistence: PersistenceEngine,
    identity: SovereignIdentity,
    settings: &SlackChannelConfig,
) -> Result<(), String> {
    let engine = persistence.clone();
    let identity_for_access = identity.clone();
    let connector_id_for_access = settings.connector_id.clone();
    let access = tauri::async_runtime::spawn_blocking(move || {
        crate::native_app_ports::resolve_slack_credential(
            &engine,
            &connector_id_for_access,
            &identity_for_access,
        )
    })
    .await
    .map_err(|error| error.to_string())??;
    let connector_id = access.connector_id.clone();
    let socket_url = tauri::async_runtime::spawn_blocking(move || {
        crate::native_app_ports::open_slack_socket(&connector_id, &identity)
    })
    .await
    .map_err(|error| error.to_string())??;
    let (stream, _) = connect_async(&socket_url)
        .await
        .map_err(|_| "slack_socket_unreachable".to_string())?;
    let (mut writer, mut reader) = stream.split();
    set_worker_status(
        &inner_ref,
        "slack",
        "running",
        active_connection_state("slack"),
        Some("ready"),
    );
    while let Some(message) = reader.next().await {
        let message = message.map_err(|_| "slack_socket_read_failed".to_string())?;
        match message {
            WebSocketMessage::Text(text) => {
                let payload = serde_json::from_str::<Value>(text.as_ref())
                    .map_err(|_| "slack_socket_payload_invalid".to_string())?;
                if let Some(envelope_id) = payload.get("envelope_id").and_then(Value::as_str) {
                    writer
                        .send(WebSocketMessage::Text(
                            json!({"envelope_id": envelope_id}).to_string().into(),
                        ))
                        .await
                        .map_err(|_| "slack_socket_ack_failed".to_string())?;
                }
                if payload.get("type").and_then(Value::as_str) == Some("disconnect") {
                    return Err("slack_socket_refresh_requested".to_string());
                }
                if let Some(incoming) = slack_event_to_gateway_message(&payload, settings) {
                    if incoming_sender.send(incoming).await.is_err() {
                        return Err("slack_ingress_queue_closed".to_string());
                    }
                }
            }
            WebSocketMessage::Close(_) => return Err("slack_socket_closed".to_string()),
            WebSocketMessage::Ping(bytes) => writer
                .send(WebSocketMessage::Pong(bytes))
                .await
                .map_err(|_| "slack_socket_pong_failed".to_string())?,
            WebSocketMessage::Binary(_)
            | WebSocketMessage::Pong(_)
            | WebSocketMessage::Frame(_) => {}
        }
    }
    Err("slack_socket_closed".to_string())
}

fn slack_event_to_gateway_message(
    envelope: &Value,
    settings: &SlackChannelConfig,
) -> Option<GatewayIncomingMessage> {
    if envelope.get("type").and_then(Value::as_str) != Some("events_api") {
        return None;
    }
    let payload = envelope.get("payload")?;
    let event = payload.get("event")?;
    if event.get("bot_id").is_some() || event.get("subtype").is_some() {
        return None;
    }
    if !matches!(
        event.get("type").and_then(Value::as_str),
        Some("app_mention") | Some("message")
    ) {
        return None;
    }
    let sender_id = event.get("user").and_then(Value::as_str)?.trim();
    let channel_id = event.get("channel").and_then(Value::as_str)?.trim();
    let event_type = event.get("type").and_then(Value::as_str)?;
    let raw_body = event.get("text").and_then(Value::as_str)?.trim();
    let body = if event_type == "app_mention" && raw_body.starts_with("<@") {
        raw_body
            .find('>')
            .map(|index| raw_body[index + 1..].trim())
            .unwrap_or(raw_body)
    } else {
        raw_body
    };
    if sender_id != settings.owner_id
        || !settings.allowlist_channels.contains(channel_id)
        || body.is_empty()
    {
        return None;
    }
    let message_id = payload
        .get("event_id")
        .and_then(Value::as_str)
        .or_else(|| event.get("client_msg_id").and_then(Value::as_str))
        .or_else(|| event.get("ts").and_then(Value::as_str))?
        .to_string();
    Some(GatewayIncomingMessage {
        platform: "slack".to_string(),
        sender_id: sender_id.to_string(),
        sender_display_name: None,
        channel_id: Some(channel_id.to_string()),
        body: body.to_string(),
        message_id: Some(message_id),
        received_at_ms: unix_time_ms(),
        requested_actions: Vec::new(),
    })
}

pub(super) async fn send_slack_message(
    client: &HttpClient,
    bot_token: &str,
    channel_id: &str,
    body: &str,
) -> Result<String, String> {
    let response = client
        .post("https://slack.com/api/chat.postMessage")
        .bearer_auth(bot_token)
        .json(&json!({"channel": channel_id, "text": body}))
        .send()
        .await
        .map_err(|_| "slack_connection_unreachable".to_string())?;
    let status = response.status();
    let payload = response
        .json::<Value>()
        .await
        .map_err(|_| "slack_response_invalid".to_string())?;
    if !status.is_success() || payload.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(payload
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("slack_message_rejected")
            .to_string());
    }
    payload
        .get("ts")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "slack_message_receipt_missing".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> SlackChannelConfig {
        SlackChannelConfig {
            connector_id: "connector-slack".to_string(),
            owner_id: "U123".to_string(),
            allowlist_channels: HashSet::from(["C123".to_string()]),
        }
    }

    #[test]
    fn inbound_events_require_the_approved_owner_and_channel() {
        let envelope = json!({
            "type": "events_api",
            "payload": {
                "event_id": "Ev1",
                "event": {"type":"app_mention","user":"U123","channel":"C123","text":"hello"}
            }
        });
        let accepted = slack_event_to_gateway_message(&envelope, &settings()).unwrap();
        assert_eq!(accepted.body, "hello");
        let wrong_owner = json!({
            "type": "events_api",
            "payload": {
                "event_id": "Ev2",
                "event": {"type":"app_mention","user":"U999","channel":"C123","text":"hello"}
            }
        });
        assert!(slack_event_to_gateway_message(&wrong_owner, &settings()).is_none());
    }

    #[test]
    fn app_mention_markup_is_not_forwarded_to_the_model() {
        let envelope = json!({
            "type": "events_api",
            "payload": {
                "event_id": "Ev3",
                "event": {
                    "type":"app_mention",
                    "user":"U123",
                    "channel":"C123",
                    "text":"<@UOOMU> summarize this"
                }
            }
        });
        assert_eq!(
            slack_event_to_gateway_message(&envelope, &settings())
                .unwrap()
                .body,
            "summarize this"
        );
    }

    #[test]
    fn channel_settings_require_one_real_connector_and_allowlist() {
        let config = ChannelConfigRecord {
            platform: "slack".to_string(),
            label: "Slack".to_string(),
            is_active: true,
            credentials_json: json!({
                "connectorId": "connector-slack",
                "allowlistChannels": ["C123", "D123"]
            })
            .to_string(),
            owner_id: Some("U123".to_string()),
            updated_at_ms: 1,
        };
        let parsed = slack_config_from_channel(&config).unwrap();
        assert_eq!(parsed.connector_id, "connector-slack");
        assert_eq!(parsed.allowlist_channels.len(), 2);
    }
}
