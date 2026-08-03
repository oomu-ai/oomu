use super::repository::set_project_binding;
use crate::{
    db::PersistenceEngine,
    projects::repository::{ensure_internal_local_files_project, INTERNAL_LOCAL_FILES_PROJECT_ID},
};

#[test]
fn connector_binding_rejects_the_private_local_files_project() {
    let root = std::env::temp_dir().join(format!(
        "oomu-reserved-project-connector-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let connection = engine.open_connection().unwrap();
    ensure_internal_local_files_project(&connection).unwrap();
    drop(connection);

    assert_eq!(
        set_project_binding(
            &engine,
            "connector_00000000-0000-4000-8000-000000000001",
            INTERNAL_LOCAL_FILES_PROJECT_ID,
            true,
        )
        .unwrap_err(),
        "This private OOMU workspace is managed automatically."
    );

    let _ = std::fs::remove_dir_all(root);
}
