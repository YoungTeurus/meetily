CREATE TABLE automatic_retranscription_settings (
    id INTEGER PRIMARY KEY CHECK(id=1),
    config TEXT NOT NULL
);
INSERT INTO automatic_retranscription_settings VALUES(1,'{"enabled":false,"provider":"whisper","model":"","language":"auto"}');
CREATE TABLE automatic_retranscription_jobs (
    job_id TEXT PRIMARY KEY,
    recording_id TEXT NOT NULL UNIQUE,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    state TEXT NOT NULL,
    config TEXT NOT NULL,
    error TEXT,
    result TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
