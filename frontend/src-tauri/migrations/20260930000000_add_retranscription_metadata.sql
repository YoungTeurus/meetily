-- Hints are opt-in settings; run-specific terms never mutate this table.
CREATE TABLE transcription_vocabulary (
    id TEXT PRIMARY KEY CHECK (id = 'global'),
    vocabulary TEXT NOT NULL
);
CREATE TABLE meeting_transcription_metadata (
    meeting_id TEXT PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE,
    transcript_revision INTEGER NOT NULL DEFAULT 0,
    summary_stale INTEGER NOT NULL DEFAULT 0
);
