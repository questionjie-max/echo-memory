-- Keep every FTS row aligned when a record changes knowledge base or title.
DROP TRIGGER IF EXISTS records_search_update;
CREATE TRIGGER records_search_update AFTER UPDATE OF title, project_id ON records BEGIN
  UPDATE record_search
  SET project_id = new.project_id, title = new.title
  WHERE record_id = new.id;
END;

CREATE TABLE IF NOT EXISTS app_settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

INSERT OR IGNORE INTO app_settings (key, value, updated_at)
VALUES ('mcp_enabled', 'false', CURRENT_TIMESTAMP);
