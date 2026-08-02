-- Document records reuse the existing managed-path/hash columns and transcript pipeline.
-- This compound index keeps duplicate checks scoped by source kind without preventing
-- an explicitly confirmed duplicate from being imported as a separate record.
CREATE INDEX IF NOT EXISTS records_source_hash_idx
    ON records(source_type, audio_hash);
