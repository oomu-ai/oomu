use super::McpClientRegistry;
use crate::native_app_ports::{
    ConnectedToolCapability, ConnectedToolCatalogFuture, ConnectedToolCatalogPort,
    ConnectedToolCatalogSource,
};
use tauri::Manager;

impl ConnectedToolCatalogSource for McpClientRegistry {
    fn connected_tool_catalog(&self) -> ConnectedToolCatalogFuture<'_> {
        Box::pin(async move {
            McpClientRegistry::connected_tool_catalog(self)
                .await
                .into_iter()
                .map(|(server_name, tool)| ConnectedToolCapability {
                    server_name,
                    tool_name: tool.name,
                    description: tool.description,
                    input_schema: tool.input_schema,
                })
                .collect()
        })
    }
}

pub(crate) fn install_connected_tool_catalog_port(app: &tauri::AppHandle) {
    if app.try_state::<ConnectedToolCatalogPort>().is_some() {
        return;
    }
    let registry = app.state::<McpClientRegistry>().inner().clone();
    let _ = app.manage(ConnectedToolCatalogPort::new(registry));
}
