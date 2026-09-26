use super::LibraryRepository;
use crate::error::{AppError, AppResult};
use crate::types::Project;
use chrono::Utc;
use rusqlite::{params, OptionalExtension, Row};
use uuid::Uuid;

impl LibraryRepository {
    pub fn create_project(&self, name: &str) -> AppResult<Project> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::Invalid("项目名不能为空".into()));
        }
        let now = Utc::now().to_rfc3339();
        let project = Project {
            id: Uuid::new_v4().to_string(),
            name: name.to_owned(),
            status: "active".to_owned(),
            created_at: now.clone(),
            updated_at: now,
        };
        self.connect()?.execute(
            "INSERT INTO projects (id, name, status, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![project.id, project.name, project.status, project.created_at, project.updated_at],
        )?;
        Ok(project)
    }

    pub fn list_projects(&self) -> AppResult<Vec<Project>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, name, status, created_at, updated_at FROM projects ORDER BY updated_at DESC",
        )?;
        let rows = statement
            .query_map([], Self::map_project)?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    pub fn get_project(&self, id: &str) -> AppResult<Project> {
        self.connect()?
            .query_row(
                "SELECT id, name, status, created_at, updated_at FROM projects WHERE id = ?1",
                params![id],
                Self::map_project,
            )
            .optional()?
            .ok_or_else(|| AppError::NotFound(format!("project {id}")))
    }

    /// 更新项目名与状态（传 None 表示保持原值）。
    pub fn update_project(
        &self,
        id: &str,
        name: Option<&str>,
        status: Option<&str>,
    ) -> AppResult<Project> {
        let current = self.get_project(id)?;
        let new_name = match name {
            Some(n) if n.trim().is_empty() => {
                return Err(AppError::Invalid("项目名不能为空".into()))
            }
            Some(n) => n.trim().to_owned(),
            None => current.name,
        };
        let new_status = match status {
            Some(s) if s != "active" && s != "archived" => {
                return Err(AppError::Invalid(format!("非法项目状态: {s}")))
            }
            Some(s) => s.to_owned(),
            None => current.status,
        };
        let now = Utc::now().to_rfc3339();
        self.connect()?.execute(
            "UPDATE projects SET name = ?2, status = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, new_name, new_status, now],
        )?;
        self.get_project(id)
    }

    /// 删除项目。关联记录的 project_id 由外键 ON DELETE SET NULL 置空。
    pub fn delete_project(&self, id: &str) -> AppResult<()> {
        let affected = self
            .connect()?
            .execute("DELETE FROM projects WHERE id = ?1", params![id])?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("project {id}")));
        }
        Ok(())
    }

    fn map_project(row: &Row<'_>) -> rusqlite::Result<Project> {
        Ok(Project {
            id: row.get(0)?,
            name: row.get(1)?,
            status: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
        })
    }
}
