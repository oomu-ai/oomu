PRAGMA foreign_keys = OFF;

DROP TRIGGER IF EXISTS validate_whatsapp_owner_on_insert;
DROP TRIGGER IF EXISTS validate_whatsapp_owner_on_update;

ALTER TABLE channel_configs RENAME TO channel_configs_before_first_party_channels;

CREATE TABLE channel_configs (
    platform TEXT PRIMARY KEY
        CHECK(platform IN ('signal', 'whatsapp', 'telegram', 'discord', 'slack')),
    is_active INTEGER NOT NULL DEFAULT 0 CHECK(is_active IN (0, 1)),
    credentials_json TEXT NOT NULL DEFAULT '{}',
    owner_id TEXT NOT NULL DEFAULT '',
    updated_at_ms INTEGER NOT NULL DEFAULT 0,
    encryption_state TEXT NOT NULL DEFAULT '{}'
);

INSERT INTO channel_configs (
    platform,
    is_active,
    credentials_json,
    owner_id,
    updated_at_ms,
    encryption_state
)
SELECT
    platform,
    is_active,
    credentials_json,
    owner_id,
    updated_at_ms,
    encryption_state
FROM channel_configs_before_first_party_channels;

DROP TABLE channel_configs_before_first_party_channels;

INSERT INTO channel_configs (
    platform,
    is_active,
    credentials_json,
    owner_id,
    updated_at_ms,
    encryption_state
)
VALUES ('slack', 0, '{}', '', 0, '{}')
ON CONFLICT(platform) DO NOTHING;

PRAGMA foreign_keys = ON;
