PRAGMA foreign_keys = ON;

-- Releases before this migration could retain more than one interrupted
-- execution for the same immutable plan/turn origin. Preserve the newest
-- execution as the resumable authority and retire older duplicates without
-- deleting their history.
INSERT INTO agent_execution_logs (
    execution_id,
    plan_id,
    session_id,
    agent_id,
    level,
    phase,
    message,
    payload_json,
    created_at_ms,
    encryption_state
)
SELECT
    duplicate.execution_id,
    duplicate.plan_id,
    duplicate.session_id,
    duplicate.agent_id,
    'warn',
    'cancelled',
    'A newer execution for this same turn was retained during a verified startup migration.',
    '{"code":"duplicate_agent_execution_origin_retired","recoverable":false}',
    duplicate.updated_at_ms,
    duplicate.encryption_state
FROM agent_executions AS duplicate
WHERE duplicate.status IN ('running', 'halted')
  AND EXISTS (
      SELECT 1
      FROM agent_executions AS keeper
      WHERE keeper.plan_id = duplicate.plan_id
        AND keeper.turn_id = duplicate.turn_id
        AND keeper.generation_token = duplicate.generation_token
        AND keeper.status IN ('running', 'halted')
        AND (
            keeper.updated_at_ms > duplicate.updated_at_ms
            OR (
                keeper.updated_at_ms = duplicate.updated_at_ms
                AND keeper.created_at_ms > duplicate.created_at_ms
            )
            OR (
                keeper.updated_at_ms = duplicate.updated_at_ms
                AND keeper.created_at_ms = duplicate.created_at_ms
                AND keeper.execution_id > duplicate.execution_id
            )
        )
  );

UPDATE agent_executions AS duplicate
SET status = 'cancelled'
WHERE duplicate.status IN ('running', 'halted')
  AND EXISTS (
      SELECT 1
      FROM agent_executions AS keeper
      WHERE keeper.plan_id = duplicate.plan_id
        AND keeper.turn_id = duplicate.turn_id
        AND keeper.generation_token = duplicate.generation_token
        AND keeper.status IN ('running', 'halted')
        AND (
            keeper.updated_at_ms > duplicate.updated_at_ms
            OR (
                keeper.updated_at_ms = duplicate.updated_at_ms
                AND keeper.created_at_ms > duplicate.created_at_ms
            )
            OR (
                keeper.updated_at_ms = duplicate.updated_at_ms
                AND keeper.created_at_ms = duplicate.created_at_ms
                AND keeper.execution_id > duplicate.execution_id
            )
        )
  );

CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_executions_active_plan_origin
ON agent_executions(plan_id, turn_id, generation_token)
WHERE status IN ('running', 'halted');
