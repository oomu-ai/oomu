//! Neutral, typed ports for Shield-approved local and connected-app actions.
//!
//! Ports cannot select a transport, server, or tool. Concrete adapters remain
//! responsible for trusted-built-in, local-only, and connector enforcement.
use serde::Serialize;
use serde_json::Value;
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, OnceLock},
};

use crate::{db::PersistenceEngine, sovereign_identity::SovereignIdentity};

pub(crate) type LocalMailFuture<'a> =
    Pin<Box<dyn Future<Output = Result<LocalMailReceipt, String>> + Send + 'a>>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MailDraftContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) bcc: Option<String>,
    pub(crate) subject: String,
    pub(crate) body: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MailDraftRequest {
    pub(crate) content: MailDraftContent,
    pub(crate) reuse_existing_matching: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MailDraftPostconditionRequest {
    pub(crate) content: MailDraftContent,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MailSendRequest {
    pub(crate) to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) bcc: Option<String>,
    pub(crate) subject: String,
    pub(crate) body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) attachment_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LocalMailReceipt {
    pub(crate) is_error: bool,
    pub(crate) structured_content: Option<Value>,
}

pub(crate) trait LocalApplicationMailPort {
    fn create_mail_draft<'a>(&'a self, request: MailDraftRequest) -> LocalMailFuture<'a>;

    fn verify_mail_draft<'a>(
        &'a self,
        request: MailDraftPostconditionRequest,
    ) -> LocalMailFuture<'a>;

    fn send_mail<'a>(&'a self, request: MailSendRequest) -> LocalMailFuture<'a>;
}

#[derive(Clone, Debug)]
pub(crate) struct ConnectedToolCapability {
    pub(crate) server_name: String,
    pub(crate) tool_name: String,
    pub(crate) description: String,
    pub(crate) input_schema: Value,
}

pub(crate) type ConnectedToolCatalogFuture<'a> =
    Pin<Box<dyn Future<Output = Vec<ConnectedToolCapability>> + Send + 'a>>;

pub(crate) trait ConnectedToolCatalogSource: Send + Sync {
    fn connected_tool_catalog(&self) -> ConnectedToolCatalogFuture<'_>;
}

#[derive(Clone)]
pub(crate) struct ConnectedToolCatalogPort {
    source: Arc<dyn ConnectedToolCatalogSource>,
}

impl ConnectedToolCatalogPort {
    pub(crate) fn new(source: impl ConnectedToolCatalogSource + 'static) -> Self {
        Self {
            source: Arc::new(source),
        }
    }

    pub(crate) async fn connected_tool_catalog(&self) -> Vec<ConnectedToolCapability> {
        self.source.connected_tool_catalog().await
    }
}

#[derive(Clone)]
pub(crate) struct SlackGatewayCredential {
    pub connector_id: String,
    pub bot_access_token: String,
}

pub(crate) struct SlackConnectorPort {
    pub resolve_credential:
        fn(&PersistenceEngine, &str, &SovereignIdentity) -> Result<SlackGatewayCredential, String>,
    pub open_socket: fn(&str, &SovereignIdentity) -> Result<String, String>,
}

static SLACK_CONNECTOR_PORT: OnceLock<SlackConnectorPort> = OnceLock::new();

pub(crate) fn install_slack_connector(port: SlackConnectorPort) {
    let _ = SLACK_CONNECTOR_PORT.set(port);
}

pub(crate) fn resolve_slack_credential(
    engine: &PersistenceEngine,
    connector_id: &str,
    identity: &SovereignIdentity,
) -> Result<SlackGatewayCredential, String> {
    (slack_connector_port()?.resolve_credential)(engine, connector_id, identity)
}

pub(crate) fn open_slack_socket(
    connector_id: &str,
    identity: &SovereignIdentity,
) -> Result<String, String> {
    (slack_connector_port()?.open_socket)(connector_id, identity)
}

fn slack_connector_port() -> Result<&'static SlackConnectorPort, String> {
    SLACK_CONNECTOR_PORT
        .get()
        .ok_or_else(|| "slack_connector_port_unavailable".to_string())
}
