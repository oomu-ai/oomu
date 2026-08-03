use super::*;
use crate::p0_contracts::{
    EvidenceClass, P0EventEnvelope, ProjectId, TaskId, TaskRunId, P0_CONTRACT_VERSION,
};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn deterministic_package_contains_editable_native_objects_and_notes() {
    let fixture = deterministic_presentation_fixture();
    let first = build_presentation(&fixture).unwrap();
    let second = build_presentation(&fixture).unwrap();
    assert_eq!(first.bytes, second.bytes);
    assert_eq!(first.package_sha256, second.package_sha256);
    let entries = super::zip::read_zip(&first.bytes).unwrap();
    let slide = String::from_utf8(entries["ppt/slides/slide1.xml"].clone()).unwrap();
    assert!(slide.contains("<p:sp>"));
    assert!(slide.contains("drawingml/2006/table"));
    assert!(slide.contains("drawingml/2006/chart"));
    assert!(entries.contains_key("ppt/charts/chart1.xml"));
    assert!(entries.contains_key("ppt/notesSlides/notesSlide1.xml"));
    assert!(entries.contains_key("ppt/slideMasters/slideMaster1.xml"));
}

#[test]
fn table_headers_have_explicit_viewer_independent_fill() {
    let mut fixture = deterministic_presentation_fixture();
    let ElementContent::Table { table } = &mut fixture.slides[0].elements[2].content else {
        panic!("fixture changed")
    };
    for run in table.rows[0]
        .iter_mut()
        .flat_map(|block| &mut block.paragraphs)
        .flat_map(|paragraph| &mut paragraph.runs)
    {
        run.color = "FFFFFF".to_string();
    }
    let built = build_presentation(&fixture).unwrap();
    let entries = super::zip::read_zip(&built.bytes).unwrap();
    let slide = String::from_utf8(entries["ppt/slides/slide1.xml"].clone()).unwrap();
    let explicit_header_fill =
        r#"<a:tcPr><a:solidFill><a:srgbClr val="17365D"/></a:solidFill></a:tcPr>"#;

    assert_eq!(
        slide.matches(explicit_header_fill).count(),
        2,
        "each header cell must carry a direct dark fill instead of relying on a viewer's table style"
    );
    assert!(
        slide.contains("<a:tcPr/>"),
        "body cells remain independently styled"
    );

    let dark_text = build_presentation(&deterministic_presentation_fixture()).unwrap();
    let dark_text_entries = super::zip::read_zip(&dark_text.bytes).unwrap();
    let dark_text_slide =
        String::from_utf8(dark_text_entries["ppt/slides/slide1.xml"].clone()).unwrap();
    let explicit_light_fill =
        r#"<a:tcPr><a:solidFill><a:srgbClr val="D9E2F3"/></a:solidFill></a:tcPr>"#;
    assert_eq!(dark_text_slide.matches(explicit_light_fill).count(), 2);
}

#[test]
fn policy_is_fail_closed_and_normalizes_only_selected_actions() {
    let mut fixture = deterministic_presentation_fixture();
    let ElementContent::TextBox { text } = &mut fixture.slides[0].elements[0].content else {
        panic!("fixture changed")
    };
    text.paragraphs[0].runs[0].font_family = "Unregistered Font".to_string();
    assert!(validate_presentation(&fixture)
        .unwrap_err()
        .contains("disallowed font"));

    fixture.policy.missing_font = MissingFontPolicy::SubstituteTheme;
    let normalized = apply_presentation_policies(&fixture).unwrap();
    let ElementContent::TextBox { text } = &normalized.presentation.slides[0].elements[0].content
    else {
        panic!("fixture changed")
    };
    assert_eq!(text.paragraphs[0].runs[0].font_family, "Arial");
    assert!(normalized
        .notices
        .iter()
        .any(|notice| notice.code == "font_substituted"));

    fixture.template.imported = true;
    fixture.template.template_id = None;
    fixture.template.fingerprint_sha256 = "forged".to_string();
    assert!(validate_presentation(&fixture).is_err());
}

#[test]
fn long_localized_text_fails_closed_before_delivery() {
    let mut fixture = deterministic_presentation_fixture();
    fixture.locale = "ja-JP".to_string();
    let ElementContent::TextBox { text } = &mut fixture.slides[0].elements[1].content else {
        panic!("fixture changed")
    };
    text.paragraphs[0].runs[0].text =
        "重要な四半期の結果と次の行動を、読みやすく正確に説明します。".repeat(80);
    assert!(apply_presentation_policies(&fixture).is_err());
}

#[test]
fn scoped_revision_preserves_unrelated_slides_and_template_identity() {
    let mut base = deterministic_presentation_fixture();
    let mut second = base.slides[0].clone();
    second.slide_id = "slide-details".to_string();
    second.title = Some("Details".to_string());
    for element in &mut second.elements {
        element.object_id = format!("details-{}", element.object_id);
    }
    base.slides.push(second);
    validate_presentation(&base).unwrap();

    let mut candidate = base.clone();
    candidate.revision = 2;
    candidate.slides[0].title = Some("Revised operating summary".to_string());
    let request = RevisePresentationScopeRequest {
        presentation_id: crate::p0_contracts::ArtifactId::new().to_string(),
        expected_revision: 1,
        scope: PresentationRevisionScope::Slide,
        target_slide_ids: vec!["slide-summary".to_string()],
        target_object_ids: Vec::new(),
        change_summary: "Clarify summary".to_string(),
        presentation: candidate.clone(),
    };
    let revised = revise_presentation_scope_ir(&base, &request).unwrap();
    assert_eq!(revised.slides[1], base.slides[1]);
    assert_eq!(revised.template, base.template);

    candidate.slides[1].title = Some("Unauthorized change".to_string());
    let bad = RevisePresentationScopeRequest {
        presentation: candidate,
        ..request
    };
    assert!(revise_presentation_scope_ir(&base, &bad)
        .unwrap_err()
        .contains("unrelated slide"));
}

#[test]
fn element_revision_changes_only_its_explicit_target() {
    let base = deterministic_presentation_fixture();
    let target = base.slides[0].elements[0].object_id.clone();
    let mut candidate = base.clone();
    candidate.revision = 2;
    candidate.slides[0].elements[0].frame.x += 1;
    let request = RevisePresentationScopeRequest {
        presentation_id: crate::p0_contracts::ArtifactId::new().to_string(),
        expected_revision: 1,
        scope: PresentationRevisionScope::Element,
        target_slide_ids: vec![base.slides[0].slide_id.clone()],
        target_object_ids: vec![target],
        change_summary: "Move selected element".to_string(),
        presentation: candidate.clone(),
    };
    let revised = revise_presentation_scope_ir(&base, &request).unwrap();
    assert_eq!(revised.slides[0].elements[1], base.slides[0].elements[1]);

    candidate.slides[0].elements[1].frame.x += 1;
    let bad = RevisePresentationScopeRequest {
        presentation: candidate,
        ..request
    };
    assert!(revise_presentation_scope_ir(&base, &bad)
        .unwrap_err()
        .contains("unrelated element"));
}

#[test]
fn imported_part_revision_preserves_every_unrelated_part_exactly() {
    let built = build_presentation(&deterministic_presentation_fixture()).unwrap();
    let before = super::verification::read_safe_package(&built.bytes).unwrap();
    let original = String::from_utf8(before["ppt/slides/slide1.xml"].clone()).unwrap();
    let replacement = original.replace("Operating summary", "Updated summary");
    let changed = replace_imported_slide_parts(
        &built.bytes,
        &BTreeMap::from([(
            "ppt/slides/slide1.xml".to_string(),
            replacement.into_bytes(),
        )]),
    )
    .unwrap();
    let after = super::verification::read_safe_package(&changed).unwrap();
    for (name, bytes) in before {
        if name != "ppt/slides/slide1.xml" {
            assert_eq!(after.get(&name), Some(&bytes), "changed {name}");
        }
    }
}

#[test]
fn task_summary_template_compatibility_requires_the_exact_two_slide_mapping() {
    let mut fixture = deterministic_presentation_fixture();
    let mut second_layout = fixture.layouts[0].clone();
    second_layout.layout_id = "layout-details".to_string();
    second_layout.name = "Details".to_string();
    fixture.layouts.push(second_layout);
    fixture.masters[0]
        .layout_ids
        .push("layout-details".to_string());
    let mut second_slide = fixture.slides[0].clone();
    second_slide.slide_id = "slide-details".to_string();
    second_slide.layout_id = "layout-details".to_string();
    for element in &mut second_slide.elements {
        element.object_id = format!("details-{}", element.object_id);
    }
    fixture.slides.push(second_slide);
    let built = build_presentation(&fixture).unwrap();
    let inspection = inspect_presentation_template_bytes(&built.bytes).unwrap();
    assert!(inspection.task_summary_compatible);

    let one_slide = build_presentation(&deterministic_presentation_fixture()).unwrap();
    assert!(
        !inspect_presentation_template_bytes(&one_slide.bytes)
            .unwrap()
            .task_summary_compatible
    );
}

#[test]
fn verifier_rejects_flattened_or_corrupted_native_object_projection() {
    let fixture = deterministic_presentation_fixture();
    let built = build_presentation(&fixture).unwrap();
    let mut entries = super::zip::read_zip(&built.bytes).unwrap();
    let slide = String::from_utf8(entries["ppt/slides/slide1.xml"].clone())
        .unwrap()
        .replace("drawingml/2006/chart", "drawingml/2006/picture");
    entries.insert("ppt/slides/slide1.xml".to_string(), slide.into_bytes());
    let corrupted = super::zip::write_store_zip(&entries).unwrap();
    let error = verify_presentation_bytes(&corrupted, &fixture, &[]).unwrap_err();
    assert!(error.contains("typed projection"));
}

#[test]
fn imported_template_mapping_uses_exact_package_pages_for_export_evidence() {
    let mut source_ir = deterministic_presentation_fixture();
    source_ir.slides[0]
        .elements
        .retain(|element| !matches!(element.content, ElementContent::Chart { .. }));
    let source = build_presentation(&source_ir).unwrap();
    let mut imported_ir = source_ir;
    imported_ir.template = PresentationTemplateIdentity {
        template_id: Some("registered-template-1".to_string()),
        name: "Registered template".to_string(),
        imported: true,
        fingerprint_sha256: source.package_sha256.clone(),
    };
    let built = build_presentation_from_registered_template(&source.bytes, &imported_ir).unwrap();
    let verified = super::verification::verify_imported_presentation_bytes(
        &built.bytes,
        &source.bytes,
        &built.normalized,
        &built.policy_notices,
    )
    .unwrap();
    assert!(verified.record.structurally_verified);
    assert!(verified.record.visually_verified);
    assert!(verified.record.exportable);
    assert!(verified
        .record
        .checks
        .iter()
        .any(|check| check.code == "exact_package_pages_rendered" && check.passed));
    let before = super::verification::read_safe_package(&source.bytes).unwrap();
    let after = super::verification::read_safe_package(&built.bytes).unwrap();
    assert_eq!(
        before["ppt/slideMasters/slideMaster1.xml"],
        after["ppt/slideMasters/slideMaster1.xml"]
    );
    assert_eq!(
        before["ppt/slideLayouts/slideLayout1.xml"],
        after["ppt/slideLayouts/slideLayout1.xml"]
    );
    assert_eq!(before["docProps/core.xml"], after["docProps/core.xml"]);
}

#[test]
fn unresolved_layout_placeholder_is_an_export_blocker() {
    let mut fixture = deterministic_presentation_fixture();
    fixture.layouts[0].placeholders.push(SlidePlaceholder {
        placeholder_id: "required-picture".to_string(),
        kind: PlaceholderKind::Picture,
        frame: Frame {
            x: 100_000,
            y: 100_000,
            width: 500_000,
            height: 500_000,
        },
    });
    let built = build_presentation(&fixture).unwrap();
    let verified = super::verification::verify_presentation_bytes(
        &built.bytes,
        &built.normalized,
        &built.policy_notices,
    )
    .unwrap();
    assert!(verified.record.issues.iter().any(|issue| {
        issue.code == "empty_placeholder" && issue.severity == PresentationIssueSeverity::Blocker
    }));
    assert!(!verified.record.exportable);
}

#[test]
fn provenance_rejects_unbound_evidence_and_preserves_actual_class() {
    let project = ProjectId::new();
    let task = TaskId::new();
    let run = TaskRunId::new();
    let mut fixture = deterministic_presentation_fixture();
    fixture.slides[0].elements[0]
        .provenance
        .push(ProvenanceAnchor {
            source_ref: "connector.read_verified".to_string(),
            evidence_ref: format!("task-event:{run}:7"),
            note: None,
        });
    fixture.citations.push(PresentationCitation {
        citation_id: "citation-1".to_string(),
        slide_id: "slide-summary".to_string(),
        object_id: Some("title".to_string()),
        source_ref: "connector.read_verified".to_string(),
        evidence_ref: format!("task-event:{run}:7"),
        label: "Verified connector result".to_string(),
        locator: None,
    });
    let event = P0EventEnvelope {
        schema_version: P0_CONTRACT_VERSION,
        event_type: "connector.read_verified".to_string(),
        project_id: project.clone(),
        task_id: task.clone(),
        task_run_id: Some(run.clone()),
        correlation_id: "correlation".to_string(),
        sequence: 7,
        timestamp: "2026-07-12T00:00:00.000Z".to_string(),
        evidence_class: EvidenceClass::VerifiedPostcondition,
        payload: json!({}),
    };
    let bound = super::provenance::bind_from_events(
        project.as_str(),
        task.as_str(),
        run.as_str(),
        &mut fixture,
        vec![(7, event)],
    )
    .unwrap();
    assert_eq!(
        bound[0].evidence_class,
        EvidenceClass::VerifiedPostcondition
    );

    fixture.slides[0].elements[0].provenance[0].source_ref = "forged".to_string();
    assert!(super::provenance::bind_from_events(
        project.as_str(),
        task.as_str(),
        run.as_str(),
        &mut fixture,
        Vec::new(),
    )
    .is_err());
}

#[test]
fn p1_contract_and_schema_round_trip() {
    let project = ProjectId::new();
    let task = TaskId::new();
    let run = TaskRunId::new();
    let artifact = crate::p0_contracts::ArtifactId::new();
    let contract = artifact_presentation_contract(
        project.as_str(),
        task.as_str(),
        run.as_str(),
        artifact.as_str(),
        &deterministic_presentation_fixture(),
        &[],
    )
    .unwrap();
    assert_eq!(contract.slides.len(), 1);
    let json = serde_json::to_value(&contract).unwrap();
    assert!(serde_json::from_value::<crate::p1_contracts::ArtifactPresentation>(json).is_ok());

    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection.execute_batch(PRESENTATION_SCHEMA_SQL).unwrap();
    let tables = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name LIKE 'presentation_%'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(tables, 5);
}

#[test]
fn repository_retains_prior_revisions_and_supports_selected_read_only_review() {
    let root = std::env::temp_dir().join(format!(
        "oomu-presentation-repository-{}",
        crate::p0_contracts::ArtifactId::new()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let engine = crate::db::PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let project_id = ProjectId::new().to_string();
    let task_id = TaskId::new().to_string();
    let task_run_id = TaskRunId::new().to_string();
    let fixture = deterministic_presentation_fixture();
    let request = CreatePresentationRequest {
        project_id,
        task_id,
        task_run_id,
        title: fixture.title.clone(),
        presentation: fixture.clone(),
    };
    let (presentation_id, revision) = create_presentation_record(&engine, &request, &[]).unwrap();
    assert_eq!(revision, 1);
    let mut revised = fixture;
    revised.revision = 2;
    revised.slides[0].title = Some("Revised title".to_string());
    let next = create_presentation_revision(
        &engine,
        &presentation_id,
        1,
        PresentationRevisionScope::WholePresentation,
        "Revise title",
        &revised,
        &[],
    )
    .unwrap();
    assert_eq!(next, 2);
    fail_presentation_revision(&engine, &presentation_id, 2, "qualified test failure").unwrap();
    let prior = get_presentation_record(&engine, &presentation_id, Some(1)).unwrap();
    assert_eq!(prior.selected_revision, 1);
    assert_eq!(prior.presentation.revision, 1);
    assert_eq!(prior.summary.current_revision, 2);
    assert_eq!(prior.summary.status, PresentationStatus::Failed);
    assert_eq!(prior.revision_history.len(), 2);
    drop(engine);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
#[cfg(target_os = "macos")]
fn exact_package_preview_uses_the_installed_qualified_chain() {
    let readiness = super::checker_setup::presentation_checker_readiness();
    assert_eq!(readiness.status, PresentationCheckerStatus::Ready);
    let fixture = deterministic_presentation_fixture();
    let built = build_presentation(&fixture).unwrap();
    let slide_ids = fixture
        .slides
        .iter()
        .map(|slide| slide.slide_id.clone())
        .collect::<Vec<_>>();
    let rendered =
        super::exact_package_preview::render_exact_package(&built.bytes, &slide_ids).unwrap();
    assert_eq!(rendered.previews.len(), slide_ids.len());
    assert!(rendered.check.passed);
    assert!(rendered
        .renderer_identity
        .contains(crate::artifacts::ARTIFACT_RENDERER_IDENTITY));
}
