ALTER TABLE ai_conversation_thread_bindings
ADD COLUMN writer_parked INTEGER NOT NULL DEFAULT 0 CHECK (writer_parked IN (0,1));
