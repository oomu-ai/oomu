mod auth;
mod contract;
mod discovery;
mod graph;
mod graph_response;
mod http;
mod manifest;
mod oidc;

pub(super) use auth::{exchange, probe_identity, refresh, tenant_binding_hash, ExchangeRequest};
pub(super) use contract::{
    base_scopes, merge_scopes, requested_scopes, AUTHORIZATION_ENDPOINT, LOOPBACK_REDIRECT_PORT,
    MANIFEST_ID,
};
#[cfg(test)]
pub(super) use contract::{
    OUTLOOK_MAIL_DRAFT, OUTLOOK_MAIL_READ, OUTLOOK_MAIL_SEARCH, TEAMS_DRAFT,
};
pub(super) use graph::MICROSOFT_ADAPTER;
pub(super) use manifest::descriptor;
