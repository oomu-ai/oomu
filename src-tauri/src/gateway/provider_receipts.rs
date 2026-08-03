use super::*;

const MAX_PROVIDER_MESSAGE_ID_CHARS: usize = 1_024;

#[derive(Debug)]
struct TelegramPostError {
    message: String,
    retry_without_markdown: bool,
}

#[derive(Debug, Deserialize)]
struct TelegramSendMessageResponse {
    ok: bool,
    #[serde(default)]
    result: Option<TelegramSendMessageResult>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    error_code: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TelegramSendMessageResult {
    message_id: i64,
}

#[derive(Debug, Deserialize)]
struct DiscordSendMessageResponse {
    id: String,
}

#[derive(Debug, Serialize)]
struct TelegramSendMessageRequest<'a> {
    chat_id: &'a str,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    parse_mode: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct DiscordSendMessageRequest<'a> {
    content: &'a str,
}

pub(super) async fn send_discord_reply(
    client: &HttpClient,
    bot_token: &str,
    channel_id: &str,
    text: &str,
) -> Result<(), String> {
    for chunk in discord_message_chunks(text) {
        post_discord_message(client, bot_token, channel_id, &chunk).await?;
    }
    Ok(())
}

pub(super) async fn send_discord_reply_with_receipt(
    client: &HttpClient,
    bot_token: &str,
    channel_id: &str,
    text: &str,
) -> Result<String, String> {
    let mut message_ids = Vec::new();
    for chunk in discord_message_chunks(text) {
        let message_id = post_discord_message(client, bot_token, channel_id, &chunk)
            .await?
            .ok_or_else(|| "discord_send_provider_receipt_missing".to_string())?;
        message_ids.push(message_id);
    }
    ordered_provider_receipt("discord", &message_ids)
}

async fn post_discord_message(
    client: &HttpClient,
    bot_token: &str,
    channel_id: &str,
    text: &str,
) -> Result<Option<String>, String> {
    let endpoint = format!(
        "https://discord.com/api/v10/channels/{}/messages",
        channel_id.trim()
    );
    let response = client
        .post(endpoint)
        .header("Authorization", format!("Bot {}", bot_token.trim()))
        .json(&DiscordSendMessageRequest { content: text })
        .send()
        .await
        .map_err(|error| {
            format!(
                "Discord send message request failed: {}",
                crate::redaction::redact_network_error(&error.to_string())
            )
        })?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "Discord send message failed status={} body_bytes={}",
            status.as_u16(),
            body.len()
        ));
    }
    Ok(serde_json::from_str::<DiscordSendMessageResponse>(&body)
        .ok()
        .and_then(|payload| discord_provider_message_id(&payload).ok()))
}

fn discord_message_chunks(text: &str) -> Vec<String> {
    chunk_gateway_reply(text, DISCORD_MESSAGE_CHUNK_BYTES)
}

pub(super) async fn send_telegram_reply(
    client: &HttpClient,
    bot_token: &str,
    chat_id: &str,
    text: &str,
) -> Result<(), String> {
    for chunk in telegram_message_chunks(text) {
        if let Err(error) =
            post_telegram_message(client, bot_token, chat_id, &chunk, Some("Markdown")).await
        {
            log_telegram_plaintext_retry(chat_id, &error.message);
            post_telegram_message(client, bot_token, chat_id, &chunk, None)
                .await
                .map_err(|error| error.message)?;
        }
    }
    Ok(())
}

pub(super) async fn send_telegram_reply_with_receipt(
    client: &HttpClient,
    bot_token: &str,
    chat_id: &str,
    text: &str,
) -> Result<String, String> {
    let mut message_ids = Vec::new();
    for chunk in telegram_message_chunks(text) {
        let message_id =
            match post_telegram_message(client, bot_token, chat_id, &chunk, Some("Markdown")).await
            {
                Ok(message_id) => require_telegram_provider_message_id(message_id)?,
                Err(error) if error.retry_without_markdown => {
                    log_telegram_plaintext_retry(chat_id, &error.message);
                    post_telegram_message(client, bot_token, chat_id, &chunk, None)
                        .await
                        .map_err(|error| error.message)
                        .and_then(require_telegram_provider_message_id)?
                }
                Err(error) => return Err(error.message),
            };
        message_ids.push(message_id);
    }
    ordered_provider_receipt("telegram", &message_ids)
}

fn log_telegram_plaintext_retry(chat_id: &str, error: &str) {
    eprintln!(
        "SOVEREIGN_GATEWAY_TELEGRAM_MARKDOWN_SEND_RETRY chat_id_hash={} error={}",
        keyed_gateway_correlation_hash("telegram-chat-id", chat_id),
        compact_log_text(error, 160)
    );
}

async fn post_telegram_message(
    client: &HttpClient,
    bot_token: &str,
    chat_id: &str,
    text: &str,
    parse_mode: Option<&str>,
) -> Result<Option<String>, TelegramPostError> {
    let endpoint = telegram_api_url(bot_token, "sendMessage");
    let response = client
        .post(endpoint)
        .json(&TelegramSendMessageRequest {
            chat_id,
            text,
            parse_mode,
        })
        .send()
        .await
        .map_err(|error| TelegramPostError {
            message: format!(
                "Telegram sendMessage request failed: {}",
                crate::redaction::redact_network_error(&error.to_string())
            ),
            retry_without_markdown: false,
        })?;
    let status = response.status();
    let payload = response
        .json::<TelegramSendMessageResponse>()
        .await
        .map_err(|_| TelegramPostError {
            message: "telegram_send_provider_receipt_invalid".to_string(),
            retry_without_markdown: false,
        })?;
    if status.is_success() && payload.ok {
        return Ok(telegram_provider_message_id(&payload));
    }
    let retry_without_markdown = parse_mode.is_some()
        && payload
            .description
            .as_deref()
            .is_some_and(telegram_markdown_rejection);
    Err(TelegramPostError {
        message: telegram_api_failure(
            "sendMessage",
            status.as_u16(),
            payload.error_code,
            payload.description.as_deref(),
        ),
        retry_without_markdown,
    })
}

fn telegram_markdown_rejection(description: &str) -> bool {
    let description = description.to_ascii_lowercase();
    description.contains("parse entities")
        || description.contains("can't parse")
        || description.contains("can't find end of")
}

fn telegram_message_chunks(text: &str) -> Vec<String> {
    chunk_gateway_reply(text, TELEGRAM_MESSAGE_CHUNK_BYTES)
}

fn telegram_provider_message_id(payload: &TelegramSendMessageResponse) -> Option<String> {
    payload
        .result
        .as_ref()
        .map(|result| result.message_id.to_string())
}

fn require_telegram_provider_message_id(message_id: Option<String>) -> Result<String, String> {
    let message_id =
        message_id.ok_or_else(|| "telegram_send_provider_receipt_missing".to_string())?;
    normalize_provider_message_id("telegram", &message_id)
}

fn discord_provider_message_id(payload: &DiscordSendMessageResponse) -> Result<String, String> {
    normalize_provider_message_id("discord", &payload.id)
}

pub(super) fn ordered_provider_receipt(
    platform: &str,
    message_ids: &[String],
) -> Result<String, String> {
    if message_ids.is_empty() {
        return Err(format!("{platform}_send_provider_receipt_missing"));
    }
    let message_ids = message_ids
        .iter()
        .map(|message_id| normalize_provider_message_id(platform, message_id))
        .collect::<Result<Vec<_>, _>>()?;
    serde_json::to_string(&json!({"platform":platform,"messageIds":message_ids}))
        .map_err(|_| format!("{platform}_send_provider_receipt_invalid"))
}

fn normalize_provider_message_id(platform: &str, message_id: &str) -> Result<String, String> {
    let message_id = message_id.trim();
    if message_id.is_empty()
        || message_id.chars().count() > MAX_PROVIDER_MESSAGE_ID_CHARS
        || message_id.chars().any(char::is_control)
    {
        return Err(format!("{platform}_send_provider_receipt_invalid"));
    }
    Ok(message_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_provider_ids_form_one_ordered_time_independent_receipt() {
        let ids = vec!["provider-17".to_string(), "provider-18".to_string()];
        let first = ordered_provider_receipt("discord", &ids).unwrap();
        let second = ordered_provider_receipt("discord", &ids).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            serde_json::from_str::<Value>(&first).unwrap(),
            json!({"platform":"discord","messageIds":["provider-17","provider-18"]})
        );
        assert_ne!(
            first,
            ordered_provider_receipt(
                "discord",
                &["provider-18".to_string(), "provider-17".to_string()]
            )
            .unwrap()
        );
    }

    #[test]
    fn missing_or_invalid_provider_ids_never_become_receipts() {
        assert_eq!(
            ordered_provider_receipt("telegram", &[]).unwrap_err(),
            "telegram_send_provider_receipt_missing"
        );
        assert_eq!(
            ordered_provider_receipt("slack", &["  ".to_string()]).unwrap_err(),
            "slack_send_provider_receipt_invalid"
        );
    }

    #[test]
    fn telegram_and_discord_extract_authoritative_response_ids() {
        let telegram: TelegramSendMessageResponse = serde_json::from_value(json!({
            "ok":true,
            "result":{"message_id":42}
        }))
        .unwrap();
        assert_eq!(telegram_provider_message_id(&telegram).unwrap(), "42");
        let discord: DiscordSendMessageResponse =
            serde_json::from_value(json!({"id":"1152921504606846976"})).unwrap();
        assert_eq!(
            discord_provider_message_id(&discord).unwrap(),
            "1152921504606846976"
        );
        let missing: TelegramSendMessageResponse =
            serde_json::from_value(json!({"ok":true})).unwrap();
        assert_eq!(
            require_telegram_provider_message_id(telegram_provider_message_id(&missing))
                .unwrap_err(),
            "telegram_send_provider_receipt_missing"
        );
    }

    #[test]
    fn slack_provider_id_propagates_into_the_routine_receipt() {
        let receipt = ordered_provider_receipt("slack", &["1730000000.12345".to_string()]).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&receipt).unwrap(),
            json!({"platform":"slack","messageIds":["1730000000.12345"]})
        );
    }
}
