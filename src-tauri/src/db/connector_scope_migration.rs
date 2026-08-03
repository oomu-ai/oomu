use rusqlite::Connection;

pub(super) const SQL: &str = include_str!("../../migrations/0032_connector_project_scope.sql");
pub(super) const DESCRIPTOR: super::MigrationDescriptor =
    super::sql(32, "0032_connector_project_scope", SQL);

pub(super) fn apply(connection: &Connection) -> rusqlite::Result<()> {
    super::add_column_if_missing(
        connection,
        "connector_accounts",
        "all_projects_enabled",
        "ALTER TABLE connector_accounts ADD COLUMN all_projects_enabled INTEGER NOT NULL DEFAULT 0 CHECK (all_projects_enabled IN (0,1))",
    )?;
    super::add_column_if_missing(
        connection,
        "connector_accounts",
        "project_scope_reviewed_at_ms",
        "ALTER TABLE connector_accounts ADD COLUMN project_scope_reviewed_at_ms INTEGER",
    )?;
    connection.execute(
        "UPDATE connector_accounts SET project_scope_reviewed_at_ms=updated_at_ms WHERE project_scope_reviewed_at_ms IS NULL",
        [],
    )?;
    Ok(())
}

pub(super) fn verify(connection: &Connection) -> rusqlite::Result<()> {
    super::require_columns(
        connection,
        "connector_accounts",
        &["all_projects_enabled", "project_scope_reviewed_at_ms"],
    )
}
