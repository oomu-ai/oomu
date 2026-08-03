use super::*;

pub(crate) async fn validate_channel_activation(
    platform: &str,
    credentials_json: &str,
    owner_id: Option<&str>,
) -> Result<(), String> {
    let config = activation_candidate(platform, credentials_json, owner_id)?;
    tokio::time::timeout(Duration::from_secs(10), async {
        match config.platform.as_str() {
            "telegram" => {
                let credentials = telegram_credentials_from_config(&config)?;
                probe_telegram_bot(&HttpClient::new(), &credentials.bot_token).await
            }
            "discord" => {
                let credentials = discord_credentials_from_config(&config)?;
                let response = HttpClient::new()
                    .get("https://discord.com/api/v10/users/@me")
                    .header(
                        "Authorization",
                        format!("Bot {}", credentials.bot_token.trim()),
                    )
                    .send()
                    .await
                    .map_err(|_| "discord_connection_unreachable".to_string())?;
                if response.status().is_success() {
                    Ok(())
                } else {
                    Err("discord_credentials_rejected".to_string())
                }
            }
            "slack" => slack_config_from_channel(&config).map(|_| ()),
            _ => Err("gateway_platform_unsupported".to_string()),
        }
    })
    .await
    .map_err(|_| "channel_connection_timeout".to_string())?
}

pub(crate) async fn validate_slack_channel_authority(
    platform: &str,
    credentials_json: &str,
    owner_id: Option<&str>,
    persistence: PersistenceEngine,
    identity: SovereignIdentity,
) -> Result<(), String> {
    if platform.trim().to_ascii_lowercase() != "slack" {
        return Ok(());
    }
    let config = activation_candidate(platform, credentials_json, owner_id)?;
    let settings = slack_config_from_channel(&config)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::native_app_ports::resolve_slack_credential(
            &persistence,
            &settings.connector_id,
            &identity,
        )
        .map(|_| ())
    })
    .await
    .map_err(|error| error.to_string())?
}

fn activation_candidate(
    platform: &str,
    credentials_json: &str,
    owner_id: Option<&str>,
) -> Result<ChannelConfigRecord, String> {
    let platform = platform.trim().to_ascii_lowercase();
    if !is_supported_gateway_platform(&platform) {
        return Err("gateway_platform_unsupported".to_string());
    }
    let owner_id =
        clean_optional_text(owner_id).ok_or_else(|| format!("{platform}_owner_required"))?;
    let config = ChannelConfigRecord {
        label: channel_label(&platform).to_string(),
        platform,
        is_active: true,
        credentials_json: credentials_json.to_string(),
        owner_id: Some(owner_id),
        updated_at_ms: unix_time_ms(),
    };
    match config.platform.as_str() {
        "telegram" => {
            telegram_credentials_from_config(&config)?;
        }
        "discord" => {
            discord_credentials_from_config(&config)?;
        }
        "slack" => {
            slack_config_from_channel(&config)?;
        }
        _ => return Err("gateway_platform_unsupported".to_string()),
    }
    Ok(config)
}

pub(super) async fn probe_telegram_bot(client: &HttpClient, bot_token: &str) -> Result<(), String> {
    let response = client
        .get(telegram_api_url(bot_token, "getMe"))
        .send()
        .await
        .map_err(|_| "telegram_connection_unreachable".to_string())?;
    let status = response.status();
    let payload = response
        .json::<Value>()
        .await
        .map_err(|_| "telegram_credentials_rejected".to_string())?;
    if status.is_success() && payload.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err("telegram_credentials_rejected".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_candidate_requires_real_credentials_and_an_approved_owner() {
        assert_eq!(
            activation_candidate("telegram", r#"{"botToken":"123:abc"}"#, None).unwrap_err(),
            "telegram_owner_required"
        );
        assert_eq!(
            activation_candidate("telegram", "{}", Some("42")).unwrap_err(),
            "telegram_bot_token_missing"
        );
        assert_eq!(
            activation_candidate("discord", "{}", Some("owner-1")).unwrap_err(),
            "discord_bot_token_missing"
        );
        assert!(activation_candidate(
            "discord",
            r#"{"apiKey":"bot-token","allowlistChannels":["channel-1"]}"#,
            Some("owner-1"),
        )
        .is_ok());
    }
}
