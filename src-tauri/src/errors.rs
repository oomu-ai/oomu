use serde::ser::{SerializeStruct, Serializer};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum OomuError {
    #[error("Database connection failure: {0}")]
    Database(String),
    #[error("File system I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Startup failure: {0}")]
    Startup(String),
    #[error("Startup integrity failure ({code}): {detail}")]
    StartupIntegrity { code: &'static str, detail: String },
}

impl OomuError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Database(_) => "database",
            Self::Io(_) => "io",
            Self::Startup(_) => "startup",
            Self::StartupIntegrity { code, .. } => code,
        }
    }

    pub fn boundary(&self) -> &'static str {
        match self {
            Self::Database(_) => "PersistentStateEngine",
            Self::Io(_) => "FileSystem",
            Self::Startup(_) => "TauriStartup",
            Self::StartupIntegrity { .. } => "RuntimeIntegrity",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::StartupIntegrity { .. } => "startup_integrity_recovery_required".to_string(),
            _ => self.to_string(),
        }
    }

    pub fn technical_detail(&self) -> Option<&str> {
        match self {
            Self::StartupIntegrity { detail, .. } => Some(detail),
            _ => None,
        }
    }
}

impl Serialize for OomuError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("OomuError", 4)?;
        state.serialize_field("code", self.code())?;
        state.serialize_field("boundary", self.boundary())?;
        state.serialize_field("message", &self.message())?;
        if let Some(detail) = self.technical_detail() {
            state.serialize_field("technicalDetail", detail)?;
        }
        state.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn oomu_error_serializes_for_ipc() {
        let serialized = serde_json::to_value(OomuError::Database("locked".to_string()))
            .expect("OomuError serializes");

        assert_eq!(
            serialized,
            json!({
                "code": "database",
                "boundary": "PersistentStateEngine",
                "message": "Database connection failure: locked",
            })
        );
    }

    #[test]
    fn startup_integrity_error_exposes_a_stable_code_without_primary_ui_copy() {
        let serialized = serde_json::to_value(OomuError::StartupIntegrity {
            code: "runtime_profile_invalid_production_identity",
            detail: "strict code-signature verification failed".to_string(),
        })
        .expect("startup integrity error serializes");

        assert_eq!(
            serialized,
            json!({
                "code": "runtime_profile_invalid_production_identity",
                "boundary": "RuntimeIntegrity",
                "message": "startup_integrity_recovery_required",
                "technicalDetail": "strict code-signature verification failed",
            })
        );
    }
}
