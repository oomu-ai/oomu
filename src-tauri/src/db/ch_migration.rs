pub(super) const SQL: &str = include_str!("../../migrations/0033_first_party_message_channels.sql");

pub(super) const DESCRIPTOR: super::MigrationDescriptor = super::MigrationDescriptor {
    sequence: 33,
    id: "0033_first_party_message_channels",
    source: super::MigrationSource::Sql(SQL),
    destructive: true,
};

pub(super) fn is_applied(connection: &rusqlite::Connection) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='channel_configs' AND sql LIKE '%''slack''%')",
        [],
        |row| row.get(0),
    )
}
