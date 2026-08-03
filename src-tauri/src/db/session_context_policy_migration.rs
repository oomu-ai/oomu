pub(super) const SQL: &str = include_str!("../../migrations/0037_session_context_policy.sql");
pub(super) const DESCRIPTOR: super::MigrationDescriptor =
    super::sql(37, "0037_session_context_policy", SQL);
