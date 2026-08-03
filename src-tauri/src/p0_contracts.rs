use chrono::DateTime;
use rand_core::{OsRng, RngCore};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::{fmt, str::FromStr};

pub const P0_CONTRACT_VERSION: u16 = 1;

fn new_uuid_v4() -> String {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

fn valid_uuid_suffix(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36
        || bytes[8] != b'-'
        || bytes[13] != b'-'
        || bytes[18] != b'-'
        || bytes[23] != b'-'
    {
        return false;
    }
    if !bytes.iter().enumerate().all(|(index, byte)| {
        matches!(index, 8 | 13 | 18 | 23) || byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
    }) {
        return false;
    }
    if !(b'1'..=b'8').contains(&bytes[14]) || !matches!(bytes[19], b'8' | b'9' | b'a' | b'b') {
        return false;
    }
    bytes
        .iter()
        .any(|byte| byte.is_ascii_hexdigit() && *byte != b'0')
}

macro_rules! opaque_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new() -> Self {
                Self(format!(concat!($prefix, "_{}"), new_uuid_v4()))
            }

            pub fn parse(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                let suffix = value
                    .strip_prefix(concat!($prefix, "_"))
                    .ok_or_else(|| concat!("invalid ", $prefix, " identifier").to_string())?;
                if !valid_uuid_suffix(suffix) {
                    return Err(concat!("invalid ", $prefix, " identifier").to_string());
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
            }
        }
    };
}

opaque_id!(ProjectId, "project");
opaque_id!(TaskId, "task");
opaque_id!(TaskRunId, "taskrun");
opaque_id!(ArtifactId, "artifact");
opaque_id!(ConnectorId, "connector");
opaque_id!(ChildRunId, "childrun");

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Planning,
    AwaitingApproval,
    Running,
    Blocked,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceClass {
    ModelAssertion,
    ObservedResult,
    ExecutedMutation,
    VerifiedPostcondition,
    SignedArtifact,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct P0EventEnvelope {
    pub schema_version: u16,
    pub event_type: String,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_run_id: Option<TaskRunId>,
    pub correlation_id: String,
    pub sequence: u64,
    pub timestamp: String,
    pub evidence_class: EvidenceClass,
    pub payload: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawP0EventEnvelope {
    schema_version: u16,
    event_type: String,
    project_id: ProjectId,
    task_id: TaskId,
    task_run_id: Option<TaskRunId>,
    correlation_id: String,
    sequence: u64,
    timestamp: String,
    evidence_class: EvidenceClass,
    payload: Value,
}

impl<'de> Deserialize<'de> for P0EventEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawP0EventEnvelope::deserialize(deserializer)?;
        if raw.schema_version != P0_CONTRACT_VERSION {
            return Err(D::Error::custom("unsupported P0 event envelope version"));
        }
        if raw.event_type.len() < 3
            || raw.event_type.len() > 128
            || !raw
                .event_type
                .starts_with(|character: char| character.is_ascii_lowercase())
            || !raw.event_type.chars().all(|character| {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || matches!(character, '_' | '.' | '-')
            })
        {
            return Err(D::Error::custom("invalid P0 event type"));
        }
        if raw.correlation_id.is_empty() || raw.correlation_id.len() > 128 {
            return Err(D::Error::custom("invalid P0 correlation identifier"));
        }
        if !raw.timestamp.ends_with('Z') || DateTime::parse_from_rfc3339(&raw.timestamp).is_err() {
            return Err(D::Error::custom("invalid P0 event timestamp"));
        }
        Ok(Self {
            schema_version: raw.schema_version,
            event_type: raw.event_type,
            project_id: raw.project_id,
            task_id: raw.task_id,
            task_run_id: raw.task_run_id,
            correlation_id: raw.correlation_id,
            sequence: raw.sequence,
            timestamp: raw.timestamp,
            evidence_class: raw.evidence_class,
            payload: raw.payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vectors() -> Value {
        serde_json::from_str(include_str!("../../schemas/p0-contract-v1-vectors.json"))
            .expect("shared P0 contract vectors must be valid JSON")
    }

    #[test]
    fn shared_vectors_round_trip_every_contract_class() {
        let vectors = vectors();
        assert_eq!(vectors["schemaVersion"], P0_CONTRACT_VERSION);
        let ids = &vectors["idVectors"];
        assert_eq!(
            ProjectId::parse(ids["project"].as_str().unwrap())
                .unwrap()
                .as_str(),
            ids["project"]
        );
        assert_eq!(
            TaskId::parse(ids["task"].as_str().unwrap())
                .unwrap()
                .as_str(),
            ids["task"]
        );
        assert_eq!(
            TaskRunId::parse(ids["taskRun"].as_str().unwrap())
                .unwrap()
                .as_str(),
            ids["taskRun"]
        );
        assert_eq!(
            ArtifactId::parse(ids["artifact"].as_str().unwrap())
                .unwrap()
                .as_str(),
            ids["artifact"]
        );
        assert_eq!(
            ConnectorId::parse(ids["connector"].as_str().unwrap())
                .unwrap()
                .as_str(),
            ids["connector"]
        );
        assert_eq!(
            ChildRunId::parse(ids["childRun"].as_str().unwrap())
                .unwrap()
                .as_str(),
            ids["childRun"]
        );

        for state in vectors["taskStates"].as_array().unwrap() {
            let parsed: TaskState = serde_json::from_value(state.clone()).unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap(), *state);
        }
        for class in vectors["evidenceClasses"].as_array().unwrap() {
            let parsed: EvidenceClass = serde_json::from_value(class.clone()).unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap(), *class);
        }
        let envelope: P0EventEnvelope =
            serde_json::from_value(vectors["eventEnvelope"].clone()).unwrap();
        assert_eq!(
            serde_json::to_value(envelope).unwrap(),
            vectors["eventEnvelope"]
        );
    }

    #[test]
    fn malformed_ids_and_unknown_vocabulary_fail_closed() {
        for value in vectors()["invalidIds"].as_array().unwrap() {
            let value = value.as_str().unwrap();
            assert!(ProjectId::parse(value).is_err());
            assert!(TaskId::parse(value).is_err());
            assert!(TaskRunId::parse(value).is_err());
            assert!(ArtifactId::parse(value).is_err());
            assert!(ConnectorId::parse(value).is_err());
            assert!(ChildRunId::parse(value).is_err());
        }
        assert!(serde_json::from_value::<TaskState>(json!("paused")).is_err());
        assert!(serde_json::from_value::<EvidenceClass>(json!("claimed_success")).is_err());
    }

    #[test]
    fn malformed_event_versions_timestamps_and_fields_fail_closed() {
        let mut envelope = vectors()["eventEnvelope"].clone();
        envelope["schemaVersion"] = json!(2);
        assert!(serde_json::from_value::<P0EventEnvelope>(envelope).is_err());

        let mut envelope = vectors()["eventEnvelope"].clone();
        envelope["timestamp"] = json!("2026-07-10 20:15:30");
        assert!(serde_json::from_value::<P0EventEnvelope>(envelope).is_err());

        let mut envelope = vectors()["eventEnvelope"].clone();
        envelope["untrusted"] = json!(true);
        assert!(serde_json::from_value::<P0EventEnvelope>(envelope).is_err());
    }
}
