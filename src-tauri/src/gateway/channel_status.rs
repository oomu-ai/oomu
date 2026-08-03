use crate::db::ChannelConfigRecord;

pub(super) fn channel_label(platform: &str) -> &'static str {
    match platform {
        "telegram" => "Telegram",
        "discord" => "Discord",
        "slack" => "Slack",
        _ => "Channel",
    }
}

pub(super) fn is_supported_gateway_platform(platform: &str) -> bool {
    matches!(platform, "telegram" | "discord" | "slack")
}

pub(super) fn active_connection_state(platform: &str) -> &'static str {
    match platform {
        "telegram" | "discord" | "slack" => "configured",
        _ => "active",
    }
}

pub(super) fn inactive_connection_state(platform: &str) -> &'static str {
    match platform {
        "telegram" | "discord" | "slack" => "inactive",
        _ => "inactive",
    }
}

pub(super) fn worker_fingerprint(config: &ChannelConfigRecord) -> String {
    format!(
        "{}:{}:{}",
        config.platform, config.updated_at_ms, config.is_active as u8
    )
}
