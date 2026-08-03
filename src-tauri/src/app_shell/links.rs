use tauri_plugin_opener::OpenerExt;

const OOMU_MARKETPLACE_URL: &str = "https://oomu.io/";
const OOMU_PRIVACY_POLICY_URL: &str = "https://oomu.ai/privacy.html";

#[tauri::command]
pub(crate) fn open_oomu_privacy_policy(app: tauri::AppHandle) -> Result<(), String> {
    app.opener()
        .open_url(OOMU_PRIVACY_POLICY_URL, None::<&str>)
        .map_err(|error| format!("Unable to open the fixed OOMU privacy policy URL: {error}"))
}

#[tauri::command]
pub(crate) fn open_oomu_marketplace(app: tauri::AppHandle) -> Result<(), String> {
    app.opener()
        .open_url(OOMU_MARKETPLACE_URL, None::<&str>)
        .map_err(|error| format!("Unable to open the fixed OOMU marketplace URL: {error}"))
}
