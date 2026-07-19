CREATE TABLE IF NOT EXISTS mcp_access_logs (
    id         TEXT PRIMARY KEY,
    tool_name  TEXT NOT NULL,
    record_id  TEXT,
    project_id TEXT,
    called_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS mcp_access_logs_called_at_idx ON mcp_access_logs(called_at DESC);
