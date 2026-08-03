use super::is_local_provider_id;

pub const MIN_AGENT_MAX_OUTPUT_TOKENS: usize = 1_024;
pub const MAX_AGENT_MAX_OUTPUT_TOKENS: usize = 8_192;
pub const AGENT_MAX_OUTPUT_TOKEN_STEP: usize = 1_024;
pub const DEFAULT_CLOUD_MAX_OUTPUT_TOKENS: usize = 4_096;
pub const DEFAULT_LOCAL_MAX_OUTPUT_TOKENS: usize = 2_048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudModel {
    GeminiFlash,
    GeminiThreeOne,
    ClaudeFableFive,
    GPTFiveFive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingTarget {
    Local,
    Cloud(CloudModel),
}

pub fn get_max_local_context_budget() -> usize {
    crate::sys_info::max_local_context_budget_for_telemetry(
        &crate::sys_info::fetch_host_hardware_telemetry(),
    )
}

pub fn resolve_context_budget(routing_target: &RoutingTarget, user_setting: usize) -> usize {
    match routing_target {
        RoutingTarget::Local => clamp_local_context_budget(user_setting),
        RoutingTarget::Cloud(model_type) => match model_type {
            CloudModel::GeminiFlash => user_setting.clamp(8192, 1_048_576),
            CloudModel::GeminiThreeOne => user_setting.clamp(8192, 2_097_152),
            CloudModel::ClaudeFableFive => user_setting.clamp(8192, 204_800),
            CloudModel::GPTFiveFive => user_setting.clamp(8192, 131_072),
        },
    }
}

pub fn clamp_local_context_budget(user_setting: usize) -> usize {
    user_setting.clamp(2048, get_max_local_context_budget())
}

#[cfg(test)]
pub fn determine_session_planner_routing(
    session_route: &RoutingTarget,
    _user_preference: &str,
) -> RoutingTarget {
    match session_route {
        RoutingTarget::Local => RoutingTarget::Local,
        RoutingTarget::Cloud(model_type) => RoutingTarget::Cloud(*model_type),
    }
}

pub fn default_max_output_tokens_for_provider(provider_id: &str) -> usize {
    if is_local_provider_id(provider_id) {
        DEFAULT_LOCAL_MAX_OUTPUT_TOKENS
    } else {
        DEFAULT_CLOUD_MAX_OUTPUT_TOKENS
    }
}

pub fn normalize_max_output_tokens_for_provider(provider_id: &str, tokens: usize) -> usize {
    if tokens == 0 {
        return default_max_output_tokens_for_provider(provider_id);
    }

    let snapped_tokens = tokens
        .saturating_add(AGENT_MAX_OUTPUT_TOKEN_STEP / 2)
        .saturating_div(AGENT_MAX_OUTPUT_TOKEN_STEP)
        .saturating_mul(AGENT_MAX_OUTPUT_TOKEN_STEP);
    snapped_tokens.clamp(MIN_AGENT_MAX_OUTPUT_TOKENS, MAX_AGENT_MAX_OUTPUT_TOKENS)
}
