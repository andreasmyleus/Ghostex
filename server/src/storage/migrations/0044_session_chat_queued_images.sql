-- The pictures a queued chat message carries: a JSON array of the copies the queue keeps under
-- <app data>/i/queued/<promptId>/ (session_chat_queue_attachments.rs). NULL for a text-only row.
ALTER TABLE session_chat_queued_prompts ADD COLUMN imagePaths TEXT;

PRAGMA user_version = 44;
