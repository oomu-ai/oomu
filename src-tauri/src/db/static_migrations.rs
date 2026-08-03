use super::*;

pub(super) const PRIVATE_EGRESS_CONFIRMATION_SQL: &str =
    include_str!("../../migrations/0038_private_egress_confirmation.sql");
pub(super) const PRIVATE_EGRESS_CONFIRMATION_DESCRIPTOR: MigrationDescriptor = sql(
    38,
    "0038_private_egress_confirmation",
    PRIVATE_EGRESS_CONFIRMATION_SQL,
);
pub(super) const AUTO_ROUTE_MODEL_IDENTITY_SQL: &str =
    include_str!("../../migrations/0039_auto_route_model_identity.sql");
pub(super) const AUTO_ROUTE_MODEL_IDENTITY_DESCRIPTOR: MigrationDescriptor = sql(
    39,
    "0039_auto_route_model_identity",
    AUTO_ROUTE_MODEL_IDENTITY_SQL,
);
pub(super) const EXECUTION_TRANSCRIPT_CONTINUITY_SQL: &str =
    include_str!("../../migrations/0040_execution_transcript_continuity.sql");
pub(super) const EXECUTION_TRANSCRIPT_CONTINUITY_DESCRIPTOR: MigrationDescriptor =
    MigrationDescriptor {
        sequence: 40,
        id: "0040_execution_transcript_continuity",
        source: MigrationSource::Sql(EXECUTION_TRANSCRIPT_CONTINUITY_SQL),
        destructive: true,
    };
pub(super) const AUTO_ROUTE_PROVIDER_IDENTITY_SQL: &str =
    include_str!("../../migrations/0041_auto_route_provider_identity.sql");
pub(super) const AUTO_ROUTE_PROVIDER_IDENTITY_DESCRIPTOR: MigrationDescriptor = sql(
    41,
    "0041_auto_route_provider_identity",
    AUTO_ROUTE_PROVIDER_IDENTITY_SQL,
);
pub(super) const TRUTHFUL_BACKGROUND_RUNTIME_SQL: &str =
    include_str!("../../migrations/0042_truthful_background_runtime.sql");
pub(super) const TRUTHFUL_BACKGROUND_RUNTIME_DESCRIPTOR: MigrationDescriptor = sql(
    42,
    "0042_truthful_background_runtime",
    TRUTHFUL_BACKGROUND_RUNTIME_SQL,
);

pub(super) fn verify_truthful_background_runtime(connection: &Connection) -> rusqlite::Result<()> {
    require_schema_objects(
        connection,
        "table",
        &["background_service_state", "background_runtime_receipts"],
    )?;
    require_schema_objects(
        connection,
        "index",
        &["idx_background_runtime_receipts_created"],
    )?;
    require_columns(
        connection,
        "background_service_state",
        &[
            "requested_enabled",
            "runtime_state",
            "registration_state",
            "registration_backend",
            "registration_generation",
            "process_state",
            "process_id",
            "build_number",
            "build_identity",
            "profile_class",
            "profile_generation",
            "heartbeat_at_ms",
            "heartbeat_expires_at_ms",
            "menu_visible",
        ],
    )?;
    require_columns(
        connection,
        "background_runtime_receipts",
        &["build_identity", "profile_class", "profile_generation"],
    )
}

pub(super) fn apply(
    engine: &PersistenceEngine,
    connection: &mut Connection,
    database_key: &str,
) -> rusqlite::Result<()> {
    for (migration, sql) in [
        (MIGRATIONS[16], MICROSOFT_CONNECTOR_METADATA_MIGRATION),
        (MIGRATIONS[17], VERIFIED_WORKBOOK_PIPELINE_MIGRATION),
        (MIGRATIONS[18], VERIFIED_PRESENTATION_PIPELINE_MIGRATION),
        (MIGRATIONS[19], MULTIMODAL_MEDIA_MIGRATION),
        (MIGRATIONS[20], SECURE_REMOTE_DISPATCH_MIGRATION),
        (MIGRATIONS[21], CAPABILITY_BUNDLES_MIGRATION),
        (MIGRATIONS[22], ADAPTIVE_LEARNING_MIGRATION),
        (MIGRATIONS[23], SCALED_WORK_GRAPHS_ANALYSIS_MIGRATION),
        (MIGRATIONS[24], CHAT_TURN_RESPONSE_CLAIM_MIGRATION),
        (MIGRATIONS[25], PRIVATE_DATA_EGRESS_RECEIPTS_MIGRATION),
        (MIGRATIONS[26], REMOTE_RECEIPT_ATOMICITY_MIGRATION),
        (MIGRATIONS[27], REMOTE_ARTIFACT_TRUTH_MIGRATION),
        (MIGRATIONS[28], VERIFIED_FILESYSTEM_CONTEXT_MIGRATION),
        (MIGRATIONS[29], AGENT_EXECUTION_ORIGIN_UNIQUENESS_MIGRATION),
        (MIGRATIONS[30], RECOVERABLE_CHAT_DELETION_MIGRATION),
    ] {
        engine.apply_migration(connection, database_key, migration, |transaction| {
            transaction.execute_batch(sql)
        })?;
    }
    engine.apply_migration(connection, database_key, MIGRATIONS[31], |transaction| {
        connector_scope_migration::apply(transaction)
    })?;
    engine.apply_migration(connection, database_key, MIGRATIONS[32], |transaction| {
        transaction.execute_batch(ch_migration::SQL)
    })?;
    engine.apply_migration(connection, database_key, MIGRATIONS[33], |transaction| {
        chat_session_attention_migration::apply(transaction)
    })?;
    engine.apply_migration(connection, database_key, MIGRATIONS[34], |transaction| {
        transaction.execute_batch(slack_pkce_loopback_migration::SQL)
    })?;
    engine.apply_migration(connection, database_key, MIGRATIONS[35], |transaction| {
        chat_completion_attention_migration::apply(transaction)
    })?;
    engine.apply_migration(connection, database_key, MIGRATIONS[36], |transaction| {
        transaction.execute_batch(session_context_policy_migration::SQL)
    })?;
    engine.apply_migration(connection, database_key, MIGRATIONS[37], |transaction| {
        transaction.execute_batch(PRIVATE_EGRESS_CONFIRMATION_SQL)
    })?;
    engine.apply_migration(connection, database_key, MIGRATIONS[38], |transaction| {
        transaction.execute_batch(AUTO_ROUTE_MODEL_IDENTITY_SQL)
    })?;
    engine.apply_migration(connection, database_key, MIGRATIONS[39], |transaction| {
        transaction.execute_batch(EXECUTION_TRANSCRIPT_CONTINUITY_SQL)
    })?;
    engine.apply_migration(connection, database_key, MIGRATIONS[40], |transaction| {
        transaction.execute_batch(AUTO_ROUTE_PROVIDER_IDENTITY_SQL)
    })?;
    engine.apply_migration(connection, database_key, MIGRATIONS[41], |transaction| {
        transaction.execute_batch(TRUTHFUL_BACKGROUND_RUNTIME_SQL)
    })
}
