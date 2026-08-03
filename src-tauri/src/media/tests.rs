use super::*;
use crate::{
    db::PersistenceEngine,
    projects::{CreateProjectRequest, ProjectDataPolicy},
};
use base64::{engine::general_purpose::STANDARD, Engine as _};

fn engine() -> (PersistenceEngine, String) {
    let mut random = [0_u8; 8];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut random);
    let root = std::env::temp_dir().join(format!(
        "oomu-media-test-{}-{}",
        crate::foundation::clock::unix_time_ms_i64(),
        hex::encode(random)
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let project = crate::projects::repository::create(
        &engine,
        CreateProjectRequest {
            name: "Media".into(),
            description: String::new(),
            data_policy: ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    (engine, project.project_id)
}
fn png() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([24, 42, 80, 255]));
    let mut output = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut output, image::ImageFormat::Png)
        .unwrap();
    output.into_inner()
}
#[test]
fn image_ingest_hashes_and_sanitizes_without_overwriting_original() {
    let (engine, project) = engine();
    let original = repository::ingest(
        &engine,
        IngestMediaRequest {
            project_id: project.clone(),
            task_id: None,
            task_run_id: None,
            source_kind: "screenshot".into(),
            source_reference: "Screenshot".into(),
            mime_type: "image/png".into(),
            data_base64: STANDARD.encode(png()),
            retention_mode: "project".into(),
            expires_at_ms: None,
            redaction_categories: vec![],
            routing_mode: "local_only".into(),
            provider_ids: vec![],
        },
    )
    .unwrap();
    let clean = repository::sanitize_png(
        &engine,
        &MediaAssetRequest {
            project_id: project.clone(),
            media_asset_id: original.media_asset_id.clone(),
        },
    )
    .unwrap();
    assert_ne!(clean.media_asset_id, original.media_asset_id);
    assert_eq!(repository::list(&engine, &project).unwrap().len(), 2);
}
#[test]
fn mime_confusion_and_cloud_route_are_blocked() {
    let (engine, project) = engine();
    let request = IngestMediaRequest {
        project_id: project,
        task_id: None,
        task_run_id: None,
        source_kind: "screenshot".into(),
        source_reference: "Screenshot".into(),
        mime_type: "image/png".into(),
        data_base64: STANDARD.encode(b"not png"),
        retention_mode: "project".into(),
        expires_at_ms: None,
        redaction_categories: vec![],
        routing_mode: "local_only".into(),
        provider_ids: vec!["cloud".into()],
    };
    assert!(repository::ingest(&engine, request).is_err());
}
