pub(super) const SQL: &str = include_str!("../../migrations/0035_slack_pkce_loopback.sql");
pub(super) const DESCRIPTOR: super::MigrationDescriptor = super::MigrationDescriptor {
    sequence: 35,
    id: "0035_slack_pkce_loopback",
    source: super::MigrationSource::Sql(SQL),
    destructive: true,
};
