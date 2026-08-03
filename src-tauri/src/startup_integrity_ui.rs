use crate::OomuError;
use serde_json::Value;

struct StartupIntegrityCopy {
    title: String,
    body: String,
    close: String,
}

const LOCALE_CATALOGS: &[(&str, &str)] = &[
    ("de-DE", include_str!("../../src/locales/de-DE.json")),
    ("en-US", include_str!("../../src/locales/en-US.json")),
    ("es-ES", include_str!("../../src/locales/es-ES.json")),
    ("fr-FR", include_str!("../../src/locales/fr-FR.json")),
    ("id-ID", include_str!("../../src/locales/id-ID.json")),
    ("ja-JP", include_str!("../../src/locales/ja-JP.json")),
    ("pt-BR", include_str!("../../src/locales/pt-BR.json")),
    ("ru-RU", include_str!("../../src/locales/ru-RU.json")),
    ("uk-UA", include_str!("../../src/locales/uk-UA.json")),
    ("vi-VN", include_str!("../../src/locales/vi-VN.json")),
    ("zh-CN", include_str!("../../src/locales/zh-CN.json")),
    ("zh-TW", include_str!("../../src/locales/zh-TW.json")),
];

pub(crate) fn show(error: &OomuError) {
    let OomuError::StartupIntegrity { code, .. } = error else {
        return;
    };
    let copy = copy_for(code);
    let _ = rfd::MessageDialog::new()
        .set_title(copy.title)
        .set_description(copy.body)
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::OkCustom(copy.close))
        .show();
}

fn copy_for(code: &str) -> StartupIntegrityCopy {
    let catalog = selected_catalog();
    let body_key = match code {
        "runtime_profile_invalid_production_identity" => "invalid_copy",
        "runtime_profile_production_override_rejected"
        | "runtime_profile_validation_required"
        | "runtime_profile_identity_not_authorized"
        | "runtime_profile_qualification_required"
        | "runtime_profile_identity_unrecognized" => "invalid_profile",
        "single_instance_identity_mismatch"
        | "single_instance_holder_unreadable"
        | "single_instance_activation_failed" => "another_version_open",
        _ => "generic",
    };
    StartupIntegrityCopy {
        title: text(&catalog, "title"),
        body: text(&catalog, body_key),
        close: text(&catalog, "close"),
    }
}

fn selected_catalog() -> Value {
    let preferred = preferred_locale();
    let language = preferred.split(['-', '_']).next().unwrap_or_default();
    let (_, source) = LOCALE_CATALOGS
        .iter()
        .find(|(locale, _)| locale.eq_ignore_ascii_case(&preferred))
        .or_else(|| {
            LOCALE_CATALOGS
                .iter()
                .find(|(locale, _)| locale.starts_with(language))
        })
        .or_else(|| {
            LOCALE_CATALOGS
                .iter()
                .find(|(locale, _)| *locale == "en-US")
        })
        .expect("the embedded US English startup catalog must exist");
    serde_json::from_str(source).expect("embedded startup locale JSON must be valid")
}

#[cfg(target_os = "macos")]
fn preferred_locale() -> String {
    use objc2_foundation::NSLocale;

    NSLocale::preferredLanguages()
        .firstObject()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "en-US".to_string())
}

#[cfg(not(target_os = "macos"))]
fn preferred_locale() -> String {
    std::env::var("LANG")
        .ok()
        .and_then(|value| value.split('.').next().map(str::to_string))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "en-US".to_string())
}

fn text(catalog: &Value, key: &str) -> String {
    catalog
        .get("startup_integrity")
        .and_then(|value| value.get(key))
        .and_then(Value::as_str)
        .expect("every embedded locale must provide Sprint 302 startup integrity copy")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_locale_has_complete_startup_copy() {
        for (_, source) in LOCALE_CATALOGS {
            let catalog: Value = serde_json::from_str(source).unwrap();
            for key in [
                "title",
                "invalid_copy",
                "invalid_profile",
                "another_version_open",
                "generic",
                "close",
            ] {
                assert!(!text(&catalog, key).trim().is_empty());
            }
        }
    }
}
