ALTER TABLE active_session_configs
ADD COLUMN auto_compaction_enabled INTEGER NOT NULL DEFAULT 1
CHECK(auto_compaction_enabled IN (0, 1));

ALTER TABLE active_session_configs
ADD COLUMN auto_compaction_threshold_percent INTEGER NOT NULL DEFAULT 70
CHECK(auto_compaction_threshold_percent BETWEEN 50 AND 90);
