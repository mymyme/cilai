ALTER TABLE ai_thread_generations ADD COLUMN tool_config_version INTEGER NOT NULL DEFAULT 0 CHECK (tool_config_version BETWEEN 0 AND 5);
