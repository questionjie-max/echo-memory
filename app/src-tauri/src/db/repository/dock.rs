use super::LibraryRepository;
use crate::error::AppResult;
use crate::types::{DockChat, DockMessage};
use chrono::Utc;
use rusqlite::params;
use uuid::Uuid;

impl LibraryRepository {
    pub fn latest_dock_chat(&self) -> AppResult<Option<DockChat>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, title, engine, created_at, updated_at FROM dock_chats ORDER BY updated_at DESC LIMIT 1",
        )?;
        let mut rows = statement.query([])?;
        Ok(match rows.next()? {
            Some(row) => Some(Self::row_to_dock_chat(row)?),
            None => None,
        })
    }

    pub fn create_dock_chat(&self, engine: &str, title: &str) -> AppResult<DockChat> {
        let chat = DockChat {
            id: Uuid::new_v4().to_string(),
            title: title.to_owned(),
            engine: engine.to_owned(),
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        };
        self.connect()?.execute(
            "INSERT INTO dock_chats (id, title, engine, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![chat.id, chat.title, chat.engine, chat.created_at, chat.updated_at],
        )?;
        Ok(chat)
    }

    pub fn touch_dock_chat(&self, chat_id: &str, title: Option<&str>) -> AppResult<()> {
        self.connect()?.execute(
            "UPDATE dock_chats SET updated_at = ?2, title = COALESCE(?3, title) WHERE id = ?1",
            params![chat_id, Utc::now().to_rfc3339(), title],
        )?;
        Ok(())
    }

    pub fn append_dock_message(
        &self,
        chat_id: &str,
        role: &str,
        content: &str,
        mode: &str,
    ) -> AppResult<DockMessage> {
        let message = DockMessage {
            id: Uuid::new_v4().to_string(),
            chat_id: chat_id.to_owned(),
            role: role.to_owned(),
            content: content.to_owned(),
            mode: mode.to_owned(),
            created_at: Utc::now().to_rfc3339(),
        };
        self.connect()?.execute(
            "INSERT INTO dock_messages (id, chat_id, role, content, mode, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![message.id, message.chat_id, message.role, message.content, message.mode, message.created_at],
        )?;
        Ok(message)
    }

    /// 最近若干条会话消息（旧→新），作为对话上下文与界面回放。
    pub fn list_dock_messages(&self, chat_id: &str, limit: u32) -> AppResult<Vec<DockMessage>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, chat_id, role, content, mode, created_at FROM (\
                 SELECT dock_messages.rowid AS rid, dock_messages.* FROM dock_messages WHERE chat_id = ?1 ORDER BY created_at DESC, rid DESC LIMIT ?2\
             ) ORDER BY created_at ASC, rid ASC",
        )?;
        let rows = statement
            .query_map(params![chat_id, limit], |row| {
                Ok(DockMessage {
                    id: row.get(0)?,
                    chat_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    mode: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    pub fn clear_dock_chats(&self) -> AppResult<()> {
        self.connect()?.execute("DELETE FROM dock_chats", [])?;
        Ok(())
    }

    fn row_to_dock_chat(row: &rusqlite::Row<'_>) -> rusqlite::Result<DockChat> {
        Ok(DockChat {
            id: row.get(0)?,
            title: row.get(1)?,
            engine: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
        })
    }
}
