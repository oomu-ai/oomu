use super::{
    commands::{create_artifact_internal, export_verified_artifact_to_approved_path},
    ArtifactBlock, ArtifactDocument, ArtifactMetadata, ArtifactSection, CreateArtifactRequest,
    PageControls, ParagraphStyle, ThemeTokens, ARTIFACT_DOCUMENT_SCHEMA_VERSION,
};
use crate::{
    p0_contracts::EvidenceClass,
    shield_gate::{CommandStatus, ExecuteCommandResponse},
    tools::{
        create_file_contract::{self, CreateFileBrief, CreateFileEnvelope},
        task_tool_runtime::{
            TaskToolExecutionContext, TaskToolFuture, TaskToolRegistration, TaskToolValidation,
        },
    },
};
use serde_json::{json, Value};

#[path = "agent_tool_file_io.rs"]
mod file_io;
#[path = "agent_tool_scheduled_path.rs"]
mod scheduled_path;
#[cfg(test)]
use file_io::rtf_document;
use file_io::{create_verified_text_file, verify_final_created_file};
use scheduled_path::{ensure_output_parent, resolve_registration, scheduled_workflow_task};

pub(crate) fn register_task_tool() -> Result<(), String> {
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation: "create_file",
        validate: validate_registration,
        validate_resolved: validate_registration,
        resolve: resolve_registration,
        execute: execute_registration,
        planner_context: None,
        schema: create_file_contract::schema,
        metadata: create_file_contract::METADATA,
    })
}

fn validate_registration(arguments: Value) -> Result<TaskToolValidation, String> {
    create_file_contract::validate(arguments)
}

fn execute_registration<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let request = serde_json::from_value::<CreateFileEnvelope>(arguments)
            .map_err(|_| "create_file arguments do not match the registered schema.".to_string())?;
        let execution_id = context
            .execution_id
            .ok_or_else(|| "Creating a file requires an active Task.".to_string())?;
        let task = crate::tools::task_runtime::require_agent_runtime_task(
            context.persistence,
            execution_id,
        )?;
        let project_id = task.project_id.clone();
        let mut output_parent = if scheduled_workflow_task(context.persistence, &task.task_run_id)?
        {
            Some(ensure_output_parent(
                context.persistence,
                &project_id,
                &request.file.destination_path,
            )?)
        } else {
            None
        };

        let result = if matches!(request.file.format.as_str(), "pptx" | "xlsx") {
            let app = context
                .app
                .ok_or_else(|| "Creating this file requires the desktop app.".to_string())?;
            if request.file.format == "xlsx" {
                create_verified_workbook(
                    &request.file,
                    &project_id,
                    &task.task_id,
                    &task.task_run_id,
                    context.persistence,
                    context.identity,
                    app,
                )
                .await?
            } else {
                create_verified_presentation(
                    &request.file,
                    &project_id,
                    &task.task_id,
                    &task.task_run_id,
                    context.persistence,
                    context.identity,
                    app,
                )
                .await?
            }
        } else if matches!(request.file.format.as_str(), "docx" | "pdf") {
            create_verified_document(
                &request.file,
                &project_id,
                &task.task_run_id,
                context.persistence,
                context.identity,
            )
            .await?
        } else {
            create_verified_text_file(&request.file)?
        };

        let evidence = verify_final_created_file(&request.file, &result)?;
        if let Some(parent) = output_parent.as_mut() {
            parent.commit();
        }
        crate::tools::task_runtime::record_event(
            context.persistence,
            &task.task_run_id,
            "file.created",
            EvidenceClass::VerifiedPostcondition,
            json!({
                "path": evidence.canonical_path,
                "format": evidence.format,
                "sha256": evidence.file_sha256,
                "verifiedContentSha256": evidence.verified_content_sha256,
                "byteLength": evidence.byte_length,
                "verificationMethod": evidence.verification_method,
            }),
        )?;
        let message = serde_json::to_string(&json!({
            "path":evidence.canonical_path,
            "format":evidence.format,
            "sha256":evidence.file_sha256,
            "verifiedContentSha256":evidence.verified_content_sha256,
            "byteLength":evidence.byte_length,
            "verificationMethod":evidence.verification_method,
            "title":request.file.title,
            "projectId":project_id,
            "taskRunId":task.task_run_id,
        }))
        .map_err(|error| error.to_string())?;
        Ok(ExecuteCommandResponse {
            operation: "create_file".to_string(),
            status: CommandStatus::Completed,
            message,
            metrics: None,
            claims: vec![format!(
                "CLAIM local_file_created format={} sha256={} content_sha256={} byte_length={} verification_method={} path={}",
                evidence.format,
                evidence.file_sha256,
                evidence.verified_content_sha256,
                evidence.byte_length,
                evidence.verification_method,
                evidence.canonical_path
            )],
            verified: true,
            model_used: None,
        })
    })
}

struct CreatedFile {
    path: String,
    sha256: String,
}

async fn create_verified_document(
    brief: &CreateFileBrief,
    project_id: &str,
    task_run_id: &str,
    persistence: &crate::db::PersistenceEngine,
    identity: &crate::sovereign_identity::SovereignIdentity,
) -> Result<CreatedFile, String> {
    let document = ArtifactDocument {
        schema_version: ARTIFACT_DOCUMENT_SCHEMA_VERSION,
        metadata: ArtifactMetadata {
            title: brief.title.clone(),
            subtitle: String::new(),
            author: "OOMU".to_string(),
            subject: brief.title.clone(),
            keywords: vec![brief.format.clone()],
            language: brief.locale.clone(),
        },
        theme: ThemeTokens::default(),
        page: PageControls::default(),
        header: None,
        footer: None,
        sections: vec![ArtifactSection {
            heading: brief.title.clone(),
            page_break_before: false,
            blocks: vec![ArtifactBlock::Paragraph {
                text: brief.content.clone(),
                style: ParagraphStyle::Body,
                factual: false,
                sources: Vec::new(),
            }],
        }],
    };
    let record = create_artifact_internal(
        CreateArtifactRequest {
            project_id: project_id.to_string(),
            task_run_id: task_run_id.to_string(),
            document,
        },
        persistence,
        identity,
    )
    .await?;
    let version = record.current_version;
    let verified = record
        .versions
        .iter()
        .find(|item| item.version == version)
        .is_some_and(|item| {
            if brief.format == "pdf" {
                item.verification.structurally_verified_pdf
                    && item.verification.visually_verified_pdf
            } else {
                item.verification.structurally_verified_docx
            }
        });
    if !verified {
        return Err("OOMU created the document, but its checks did not pass.".to_string());
    }
    let exported = export_verified_artifact_to_approved_path(
        &record.artifact_id,
        version,
        &brief.format,
        &brief.destination_path,
        persistence,
        identity,
    )?;
    let path = exported
        .exported_files
        .first()
        .cloned()
        .ok_or_else(|| "The checked document was not saved.".to_string())?;
    let sha256 = exported
        .hashes
        .get(&brief.format)
        .cloned()
        .ok_or_else(|| "The saved document could not be verified.".to_string())?;
    Ok(CreatedFile { path, sha256 })
}

async fn create_verified_workbook(
    brief: &CreateFileBrief,
    project_id: &str,
    task_id: &str,
    task_run_id: &str,
    persistence: &crate::db::PersistenceEngine,
    identity: &crate::sovereign_identity::SovereignIdentity,
    app: &tauri::AppHandle,
) -> Result<CreatedFile, String> {
    use super::workbooks::{
        CellValue, CreateWorkbookRequest, RecalculationState, SheetVisibility, WorkbookCell,
        WorkbookDateSystem, WorkbookIr, WorkbookPolicy, Worksheet, WorksheetBounds,
        WORKBOOK_IR_SCHEMA_VERSION,
    };
    let workbook = WorkbookIr {
        schema_version: WORKBOOK_IR_SCHEMA_VERSION,
        title: brief.title.clone(),
        locale: brief.locale.clone(),
        date_system: WorkbookDateSystem::Excel1900,
        revision: 1,
        formats: Vec::new(),
        worksheets: vec![Worksheet {
            sheet_id: "sheet1".to_string(),
            name: "Sheet1".to_string(),
            bounds: WorksheetBounds {
                row_count: 1,
                column_count: 1,
            },
            visibility: SheetVisibility::Visible,
            critical: true,
            cells: vec![WorkbookCell {
                address: "A1".to_string(),
                value: CellValue::Text {
                    value: brief.content.clone(),
                },
                format_id: None,
                comment: None,
                provenance: Vec::new(),
            }],
            merged_ranges: Vec::new(),
            column_widths: Vec::new(),
            tables: Vec::new(),
            validations: Vec::new(),
            charts: Vec::new(),
        }],
        named_ranges: Vec::new(),
        recalculation: RecalculationState::default(),
        policy: WorkbookPolicy::default(),
    };
    let review = super::workbooks::create_workbook_internal(
        CreateWorkbookRequest {
            project_id: project_id.to_string(),
            task_id: task_id.to_string(),
            task_run_id: task_run_id.to_string(),
            workbook,
        },
        persistence,
        identity,
        app,
    )
    .await
    .map_err(|error| error.message)?;
    let revision = review.current_revision;
    let exported = super::workbooks::export_workbook_revision_to_approved_path(
        &review.artifact_id,
        revision,
        &brief.destination_path,
        persistence,
        identity,
        app,
    )
    .await
    .map_err(|error| error.message)?;
    Ok(CreatedFile {
        path: exported.path,
        sha256: exported.sha256,
    })
}

async fn create_verified_presentation(
    brief: &CreateFileBrief,
    project_id: &str,
    task_id: &str,
    task_run_id: &str,
    persistence: &crate::db::PersistenceEngine,
    identity: &crate::sovereign_identity::SovereignIdentity,
    app: &tauri::AppHandle,
) -> Result<CreatedFile, String> {
    use super::presentations::{
        CreatePresentationRequest, ElementContent, Frame, PresentationAspectRatio,
        PresentationElement, PresentationIr, PresentationPolicy, PresentationSlide,
        PresentationTemplateIdentity, PresentationTheme, SlideLayout, SlideLayoutKind, SlideMaster,
        SlideNotes, TextAlignment, TextBlock, TextParagraph, TextRun, ThemeColors, ThemeFonts,
        VerticalAlignment, PRESENTATION_IR_VERSION,
    };
    let text_block = |text: String, font_size_pt: f32, bold: bool| TextBlock {
        paragraphs: vec![TextParagraph {
            runs: vec![TextRun {
                text,
                font_family: "Arial".to_string(),
                font_size_pt,
                bold,
                italic: false,
                color: "202124".to_string(),
            }],
            alignment: TextAlignment::Left,
            bullet: false,
        }],
        vertical_alignment: VerticalAlignment::Top,
    };
    let presentation = PresentationIr {
        schema_version: PRESENTATION_IR_VERSION,
        title: brief.title.clone(),
        locale: brief.locale.clone(),
        revision: 1,
        aspect_ratio: PresentationAspectRatio::Widescreen,
        theme: PresentationTheme {
            theme_id: "theme-main".to_string(),
            name: "OOMU clarity".to_string(),
            colors: ThemeColors {
                dark: "202124".to_string(),
                light: "FFFFFF".to_string(),
                accent_1: "0B57D0".to_string(),
                accent_2: "188038".to_string(),
                accent_3: "B06000".to_string(),
                accent_4: "A50E0E".to_string(),
                hyperlink: "0B57D0".to_string(),
            },
            fonts: ThemeFonts {
                heading: "Arial".to_string(),
                body: "Arial".to_string(),
            },
        },
        masters: vec![SlideMaster {
            master_id: "master-main".to_string(),
            name: "Primary master".to_string(),
            theme_id: "theme-main".to_string(),
            layout_ids: vec!["layout-content".to_string()],
        }],
        layouts: vec![SlideLayout {
            layout_id: "layout-content".to_string(),
            master_id: "master-main".to_string(),
            name: "Title and content".to_string(),
            kind: SlideLayoutKind::TitleAndContent,
            placeholders: Vec::new(),
        }],
        slides: vec![PresentationSlide {
            slide_id: "slide-1".to_string(),
            layout_id: "layout-content".to_string(),
            title: Some(brief.title.clone()),
            elements: vec![
                PresentationElement {
                    object_id: "title".to_string(),
                    frame: Frame {
                        x: 600_000,
                        y: 300_000,
                        width: 10_900_000,
                        height: 900_000,
                    },
                    content: ElementContent::TextBox {
                        text: text_block(brief.title.clone(), 28.0, true),
                    },
                    provenance: Vec::new(),
                },
                PresentationElement {
                    object_id: "content".to_string(),
                    frame: Frame {
                        x: 600_000,
                        y: 1_500_000,
                        width: 10_900_000,
                        height: 4_500_000,
                    },
                    content: ElementContent::TextBox {
                        text: text_block(brief.content.clone(), 18.0, false),
                    },
                    provenance: Vec::new(),
                },
            ],
            notes: SlideNotes::default(),
            animations: Vec::new(),
        }],
        citations: Vec::new(),
        policy: PresentationPolicy::default(),
        template: PresentationTemplateIdentity::default(),
    };
    let review = super::presentations::create_presentation_internal(
        CreatePresentationRequest {
            project_id: project_id.to_string(),
            task_id: task_id.to_string(),
            task_run_id: task_run_id.to_string(),
            title: brief.title.clone(),
            presentation,
        },
        persistence,
        identity,
        app,
    )
    .await
    .map_err(|error| error.message)?;
    let exported = super::presentations::export_presentation_revision_to_approved_path(
        &review.summary.presentation_id,
        review.summary.current_revision,
        &brief.destination_path,
        persistence,
        identity,
        app,
    )
    .await
    .map_err(|error| error.message)?;
    Ok(CreatedFile {
        path: crate::shield_gate::validate_approved_external_write_target(&brief.destination_path)
            .map_err(|error| error.message)?
            .display()
            .to_string(),
        sha256: exported.sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_accepts_bounded_pdf_and_text_requests() {
        for format in [
            "pdf", "docx", "txt", "md", "rtf", "csv", "json", "html", "xls", "xlsx", "pptx", "xml",
        ] {
            let value = json!({"file":{
                "title":"Hello World",
                "content":"Hello World",
                "locale":"en-US",
                "format":format,
                "destinationPath":format!("~/Downloads/Hello World.{format}")
            }});
            assert!(validate_registration(value).is_ok(), "{format}");
        }
    }

    #[test]
    fn registration_rejects_disguised_or_unsupported_formats() {
        assert!(validate_registration(json!({"file":{
            "title":"Report","content":"hello","locale":"en-US","format":"pdf",
            "destinationPath":"~/Downloads/report.txt"
        }}))
        .is_err());
        assert!(validate_registration(json!({"file":{
            "title":"Legacy","content":"hello","locale":"en-US","format":"xls",
            "destinationPath":"~/Downloads/legacy.xlsx"
        }}))
        .is_err());
    }

    #[test]
    fn rich_text_serializer_emits_real_rtf_and_escapes_control_text() {
        let output = rtf_document("Hello {world} \\ café");
        assert!(output.starts_with("{\\rtf1\\ansi"));
        assert!(output.contains("\\{world\\}"));
        assert!(output.contains("\\\\"));
        assert!(output.contains("\\u233?"));
    }

    #[test]
    fn legacy_excel_serializer_emits_a_verified_biff8_compound_file() {
        let bytes =
            super::super::agent_tool_legacy_xls::legacy_xls_bytes("Sheet1", "Hello World").unwrap();
        assert_eq!(
            &bytes[..8],
            &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]
        );
        super::super::agent_tool_legacy_xls::verify_legacy_xls_bytes(&bytes, "Hello World")
            .unwrap();
    }

    #[test]
    fn native_text_and_legacy_formats_are_really_written_and_hashed() {
        let root = std::env::temp_dir().join(format!(
            "oomu-native-file-writers-{}-{}",
            std::process::id(),
            crate::foundation::clock::unix_time_ms_i64()
        ));
        std::fs::create_dir_all(&root).unwrap();

        for format in ["txt", "md", "rtf", "csv", "json", "html", "xml", "xls"] {
            let destination = root.join(format!("Hello World.{format}"));
            let content = if format == "json" {
                r#"{"message":"Hello World"}"#
            } else {
                "Hello World"
            };
            let created = create_verified_text_file(&CreateFileBrief {
                title: "Hello World".to_string(),
                content: content.to_string(),
                locale: "en-US".to_string(),
                format: format.to_string(),
                destination_path: destination.display().to_string(),
            })
            .unwrap_or_else(|error| panic!("{format}: {error}"));
            assert_eq!(
                created.path,
                std::fs::canonicalize(&destination)
                    .unwrap()
                    .display()
                    .to_string()
            );
            assert_eq!(created.sha256.len(), 64);
            assert_eq!(
                created.sha256,
                crate::foundation::digest::sha256_file_hex(&destination).unwrap()
            );
            assert!(destination.metadata().unwrap().len() > 0);
        }

        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(test)]
#[path = "agent_tool_chat_execution_tests.rs"]
mod agent_tool_chat_execution_tests;
#[cfg(test)]
#[path = "agent_tool_scheduled_path_tests.rs"]
mod agent_tool_scheduled_path_tests;
