-- AI 伙伴停靠栏会话与消息。
CREATE TABLE IF NOT EXISTS dock_chats (
    id         TEXT PRIMARY KEY,
    title      TEXT NOT NULL DEFAULT '新对话',
    engine     TEXT NOT NULL DEFAULT 'local', -- local | external
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS dock_messages (
    id         TEXT PRIMARY KEY,
    chat_id    TEXT NOT NULL REFERENCES dock_chats(id) ON DELETE CASCADE,
    role       TEXT NOT NULL, -- user | assistant
    content    TEXT NOT NULL,
    mode       TEXT NOT NULL DEFAULT 'free', -- summary | creation | inspire | free
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_dock_messages_chat ON dock_messages(chat_id, created_at);
