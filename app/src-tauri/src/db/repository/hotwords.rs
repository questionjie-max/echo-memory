use super::LibraryRepository;
use crate::error::AppResult;
use crate::types::Hotword;
use chrono::Utc;
use rusqlite::params;
use uuid::Uuid;

impl LibraryRepository {
    pub fn list_hotwords(&self) -> AppResult<Vec<Hotword>> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare("SELECT id, term, note, created_at FROM hotwords ORDER BY created_at")?;
        let rows = statement
            .query_map([], |row| {
                Ok(Hotword {
                    id: row.get(0)?,
                    term: row.get(1)?,
                    note: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    /// 拼接为 Whisper initial_prompt 使用的热词前缀，限制在 160 个字符内。
    pub fn hotwords_prompt(&self) -> AppResult<String> {
        let terms: Vec<String> = self
            .list_hotwords()?
            .into_iter()
            .map(|hotword| hotword.term)
            .collect();
        if terms.is_empty() {
            return Ok(String::new());
        }
        let mut prompt = String::from("术语表：");
        for term in terms {
            if prompt.chars().count() + term.chars().count() + 1 > 160 {
                break;
            }
            prompt.push_str(&term);
            prompt.push('、');
        }
        Ok(prompt.trim_end_matches('、').to_owned())
    }

    pub fn add_hotword(&self, term: &str, note: &str) -> AppResult<Hotword> {
        let term = term.trim();
        if term.is_empty() || term.chars().count() > 40 {
            return Err(crate::error::AppError::Invalid("热词无效".to_owned()));
        }
        let hotword = Hotword {
            id: Uuid::new_v4().to_string(),
            term: term.to_owned(),
            note: note.trim().to_owned(),
            created_at: Utc::now().to_rfc3339(),
        };
        self.connect()?.execute(
            "INSERT INTO hotwords (id, term, note, created_at) VALUES (?1, ?2, ?3, ?4)              ON CONFLICT(term) DO UPDATE SET note = excluded.note",
            params![hotword.id, hotword.term, hotword.note, hotword.created_at],
        )?;
        Ok(hotword)
    }

    pub fn remove_hotword(&self, id: &str) -> AppResult<()> {
        self.connect()?
            .execute("DELETE FROM hotwords WHERE id = ?1", params![id])?;
        Ok(())
    }
}
