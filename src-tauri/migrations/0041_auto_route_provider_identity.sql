ALTER TABLE active_session_configs
ADD COLUMN local_provider_config_id TEXT;

ALTER TABLE active_session_configs
ADD COLUMN local_provider_type TEXT;

ALTER TABLE active_session_configs
ADD COLUMN local_route_generation INTEGER NOT NULL DEFAULT 0;

ALTER TABLE auto_route_baseline_backups
ADD COLUMN local_provider_config_id TEXT;

ALTER TABLE auto_route_baseline_backups
ADD COLUMN local_provider_type TEXT;

ALTER TABLE auto_route_baseline_backups
ADD COLUMN local_route_generation INTEGER NOT NULL DEFAULT 0;
