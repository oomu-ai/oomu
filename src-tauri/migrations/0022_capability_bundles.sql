CREATE TABLE capability_bundle_records (
    bundle_id TEXT NOT NULL,
    package_version TEXT NOT NULL,
    mod_id TEXT NOT NULL,
    publisher_id TEXT NOT NULL,
    publisher_name TEXT NOT NULL,
    publisher_identity_verified INTEGER NOT NULL DEFAULT 0,
    review_state TEXT NOT NULL CHECK(review_state IN ('reviewed','unreviewed','revoked')),
    compatibility_state TEXT NOT NULL CHECK(compatibility_state IN ('compatible','incompatible','unknown')),
    payload_sha256 TEXT NOT NULL,
    manifest_json TEXT NOT NULL CHECK(json_valid(manifest_json)),
    capabilities_json TEXT NOT NULL CHECK(json_valid(capabilities_json)),
    project_ids_json TEXT NOT NULL CHECK(json_valid(project_ids_json)),
    install_state TEXT NOT NULL CHECK(install_state IN ('inspected','quarantined','active','disabled','rolled_back','removed','blocked')),
    previous_version TEXT,
    installed_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(bundle_id, package_version)
);
CREATE INDEX idx_capability_bundles_state ON capability_bundle_records(install_state, updated_at_ms DESC);

CREATE TABLE capability_bundle_receipts (
    receipt_id TEXT PRIMARY KEY NOT NULL,
    bundle_id TEXT NOT NULL,
    package_version TEXT NOT NULL,
    event_kind TEXT NOT NULL,
    detail_json TEXT NOT NULL CHECK(json_valid(detail_json)),
    created_at_ms INTEGER NOT NULL
);

CREATE TABLE capability_registry_entries (
    bundle_id TEXT NOT NULL,
    package_version TEXT NOT NULL,
    catalog_revision TEXT NOT NULL,
    name TEXT NOT NULL,
    summary TEXT NOT NULL,
    category TEXT NOT NULL,
    publisher_name TEXT NOT NULL,
    review_state TEXT NOT NULL CHECK(review_state IN ('reviewed','unreviewed','revoked')),
    compatibility_state TEXT NOT NULL,
    changelog TEXT NOT NULL,
    metadata_sha256 TEXT NOT NULL,
    metadata_signature TEXT NOT NULL,
    cached_at_ms INTEGER NOT NULL,
    PRIMARY KEY(bundle_id, package_version)
);

CREATE TABLE capability_runtime_denials (
    denial_id TEXT PRIMARY KEY NOT NULL,
    bundle_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    requested_capability TEXT NOT NULL,
    declared_capabilities_json TEXT NOT NULL CHECK(json_valid(declared_capabilities_json)),
    created_at_ms INTEGER NOT NULL
);
