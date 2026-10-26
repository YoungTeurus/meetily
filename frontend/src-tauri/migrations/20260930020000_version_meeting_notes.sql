-- Reuse the existing meeting notes storage and protect autosave from stale writers.
ALTER TABLE meeting_notes ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;
