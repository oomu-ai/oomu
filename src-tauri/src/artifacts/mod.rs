mod agent_tool;
mod agent_tool_legacy_xls;
mod commands;
pub(crate) mod decision_pack;
mod exact_package_runtime;
pub mod helper;
mod package;
pub mod presentations;
mod project_chat_document;
mod repository;
mod runtime;
mod validation;
mod verifier;
pub mod workbooks;

pub(crate) use agent_tool::register_task_tool as register_file_task_tool;
pub use commands::*;
pub use project_chat_document::*;

use crate::sovereign_identity::SignatureBlock;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub const ARTIFACT_DOCUMENT_SCHEMA_VERSION: u16 = 1;
pub const ARTIFACT_BUILDER_IDENTITY: &str =
    "oomu-artifact-builder/1.0.0+ooxml-store-v1+apple-pdfkit-v1";
pub const ARTIFACT_RENDERER_IDENTITY: &str = "oomu-artifact-pdf-helper/apple-pdfkit-v1";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactDocument {
    pub schema_version: u16,
    pub metadata: ArtifactMetadata,
    #[serde(default)]
    pub theme: ThemeTokens,
    #[serde(default)]
    pub page: PageControls,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub footer: Option<String>,
    pub sections: Vec<ArtifactSection>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactMetadata {
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub language: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThemeTokens {
    pub font_family: String,
    pub body_size_pt: f32,
    pub title_size_pt: f32,
    pub heading_color: String,
    pub accent_color: String,
    pub text_color: String,
    pub background_color: String,
}

impl Default for ThemeTokens {
    fn default() -> Self {
        Self {
            font_family: "Arial".to_string(),
            body_size_pt: 10.5,
            title_size_pt: 26.0,
            heading_color: "1F2937".to_string(),
            accent_color: "2563EB".to_string(),
            text_color: "111827".to_string(),
            background_color: "FFFFFF".to_string(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageControls {
    pub size: String,
    pub orientation: String,
    pub margin_top_in: f32,
    pub margin_right_in: f32,
    pub margin_bottom_in: f32,
    pub margin_left_in: f32,
}
impl Default for PageControls {
    fn default() -> Self {
        Self {
            size: "letter".to_string(),
            orientation: "portrait".to_string(),
            margin_top_in: 1.0,
            margin_right_in: 1.0,
            margin_bottom_in: 1.0,
            margin_left_in: 1.0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactSection {
    pub heading: String,
    #[serde(default)]
    pub page_break_before: bool,
    pub blocks: Vec<ArtifactBlock>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ArtifactBlock {
    Paragraph {
        text: String,
        #[serde(default)]
        style: ParagraphStyle,
        #[serde(default)]
        factual: bool,
        #[serde(default)]
        sources: Vec<ArtifactSourceReference>,
    },
    List {
        ordered: bool,
        items: Vec<String>,
        #[serde(default)]
        factual: bool,
        #[serde(default)]
        sources: Vec<ArtifactSourceReference>,
    },
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
        #[serde(default)]
        caption: String,
        #[serde(default)]
        factual: bool,
        #[serde(default)]
        sources: Vec<ArtifactSourceReference>,
    },
    Callout {
        label: String,
        text: String,
        #[serde(default)]
        factual: bool,
        #[serde(default)]
        sources: Vec<ArtifactSourceReference>,
    },
    Citation {
        label: String,
        url: String,
        source_ref: String,
        evidence_ref: String,
    },
    PageBreak,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParagraphStyle {
    #[default]
    Body,
    Lead,
    Quote,
    Caption,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactSourceReference {
    pub source_ref: String,
    pub evidence_ref: String,
    pub url: Option<String>,
}

impl ArtifactBlock {
    pub(crate) fn sources(&self) -> &[ArtifactSourceReference] {
        match self {
            Self::Paragraph { sources, .. }
            | Self::List { sources, .. }
            | Self::Table { sources, .. }
            | Self::Callout { sources, .. } => sources,
            Self::Citation { .. } | Self::PageBreak => &[],
        }
    }
    pub(crate) fn factual(&self) -> bool {
        match self {
            Self::Paragraph { factual, .. }
            | Self::List { factual, .. }
            | Self::Table { factual, .. }
            | Self::Callout { factual, .. } => *factual,
            Self::Citation { .. } => true,
            Self::PageBreak => false,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateArtifactRequest {
    pub project_id: String,
    pub task_run_id: String,
    pub document: ArtifactDocument,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateDecisionBriefRequest {
    pub delegation_plan_id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviseArtifactRequest {
    pub artifact_id: String,
    pub project_id: String,
    pub task_run_id: String,
    pub instruction: String,
    pub document: ArtifactDocument,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactListRequest {
    pub project_id: Option<String>,
    pub task_run_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactIdRequest {
    pub artifact_id: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRecord {
    pub artifact_id: String,
    pub project_id: String,
    pub task_run_id: String,
    pub title: String,
    pub current_version: u32,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub versions: Vec<ArtifactVersion>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactVersion {
    pub version: u32,
    pub revision_instruction: Option<String>,
    pub status: String,
    pub document: ArtifactDocument,
    pub preview_pages: Vec<String>,
    pub verification: ArtifactVerification,
    pub provenance: serde_json::Value,
    pub docx_bytes: Option<u64>,
    pub pdf_bytes: Option<u64>,
    pub docx_sha256: Option<String>,
    pub pdf_sha256: Option<String>,
    pub builder_identity: String,
    pub renderer_identity: Option<String>,
    pub created_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub last_error: Option<String>,
    pub manifest_signature: Option<SignatureBlock>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ArtifactVerification {
    pub structurally_verified_docx: bool,
    pub structurally_verified_pdf: bool,
    pub visually_verified_pdf: bool,
    pub page_count: usize,
    pub warnings: Vec<String>,
    pub renderer_probe: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactPreviewRequest {
    pub artifact_id: String,
    pub version: u32,
    pub page: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChooseArtifactExportRequest {
    pub artifact_id: String,
    pub version: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportArtifactRequest {
    pub artifact_id: String,
    pub version: u32,
    pub export_grant_id: String,
    pub format: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactExportGrantView {
    pub export_grant_id: String,
    pub directory_name: String,
    pub expires_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactExportResult {
    pub exported_files: Vec<String>,
    pub hashes: HashMap<String, String>,
}

#[derive(Clone, Default)]
pub struct ArtifactRuntimeManager {
    grants: Arc<Mutex<HashMap<String, ExportGrant>>>,
}
#[derive(Clone)]
struct ExportGrant {
    artifact_id: String,
    version: u32,
    path: PathBuf,
    expires_at_ms: i64,
}
