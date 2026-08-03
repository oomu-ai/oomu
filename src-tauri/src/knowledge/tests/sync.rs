use super::*;

#[test]
fn project_knowledge_scopes_are_explicit_and_disjoint() {
    let first =
        KnowledgeScope::from_project_id("project_11111111-1111-4111-8111-111111111111").unwrap();
    let second =
        KnowledgeScope::from_project_id("project_22222222-2222-4222-8222-222222222222").unwrap();
    assert_ne!(first.project_id, second.project_id);
    assert_ne!(
        scoped_storage_path(&first, "notes.md"),
        scoped_storage_path(&second, "notes.md")
    );
    assert!(KnowledgeScope::default().project_id.is_none());
}
