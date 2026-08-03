UPDATE chat_messages
SET metadata_json = json_set(
    COALESCE(metadata_json, '{}'),
    '$.turnState',
    (
        SELECT turns.status
        FROM chat_turns turns
        WHERE turns.workspace_id = chat_messages.workspace_id
          AND turns.session_id = chat_messages.session_id
          AND turns.turn_id = json_extract(chat_messages.metadata_json, '$.turnId')
          AND turns.generation_token = json_extract(
              chat_messages.metadata_json,
              '$.generationToken'
          )
    )
)
WHERE role = 'user'
  AND json_extract(metadata_json, '$.turnState') = 'accepted'
  AND EXISTS (
      SELECT 1
      FROM chat_turns turns
      WHERE turns.workspace_id = chat_messages.workspace_id
        AND turns.session_id = chat_messages.session_id
        AND turns.turn_id = json_extract(chat_messages.metadata_json, '$.turnId')
        AND turns.generation_token = json_extract(
            chat_messages.metadata_json,
            '$.generationToken'
        )
        AND turns.status IN ('completed', 'failed', 'cancelled', 'escalated')
  );

DELETE FROM chat_messages
WHERE role = 'assistant'
  AND content LIKE 'Logical Certificate Receipt%'
  AND json_valid(COALESCE(metadata_json, ''))
  AND json_extract(metadata_json, '$.schema') = 'oomu.agent_execution_terminal.v1';
