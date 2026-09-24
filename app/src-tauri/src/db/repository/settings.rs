use super::LibraryRepository;
use crate::error::{AppError, AppResult};
use crate::types::{AnalysisTemplate, ExternalAiSettings, KnowledgeSettings, TemplateSection};
use chrono::Utc;
use rusqlite::{params, OptionalExtension, Row};
use std::path::Path;
use uuid::Uuid;

impl LibraryRepository {
    /// 读取通用 KV 设置；不存在时返回 None。
    pub fn setting_value(&self, key: &str) -> AppResult<Option<String>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare("SELECT value FROM app_settings WHERE key = ?1")?;
        let mut rows = statement.query(params![key])?;
        Ok(match rows.next()? {
            Some(row) => Some(row.get(0)?),
            None => None,
        })
    }

    pub fn set_setting_value(&self, key: &str, value: &str) -> AppResult<()> {
        self.connect()?.execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)              ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn onboarding_completed_at(&self) -> AppResult<Option<String>> {
        Ok(self
            .setting_value("onboarding_completed_at")?
            .filter(|value| !value.trim().is_empty()))
    }

    pub fn complete_onboarding(&self) -> AppResult<()> {
        self.set_setting_value("onboarding_completed_at", &Utc::now().to_rfc3339())
    }

    pub fn reset_onboarding(&self) -> AppResult<()> {
        self.set_setting_value("onboarding_completed_at", "")
    }

    pub fn knowledge_settings(&self) -> AppResult<KnowledgeSettings> {
        Ok(KnowledgeSettings {
            transcription_language: self.setting("transcription_language", "zh")?,
            whisper_model_path: self.setting("whisper_model_path", "")?,
            analysis_model: self.setting("analysis_model", "qwen2.5:7b")?,
            embedding_model: self.setting("embedding_model", "qwen3-embedding:0.6b")?,
        })
    }

    pub fn update_knowledge_settings(
        &self,
        settings: &KnowledgeSettings,
    ) -> AppResult<KnowledgeSettings> {
        let language = settings.transcription_language.trim();
        if language.is_empty() || language.len() > 16 {
            return Err(AppError::Invalid("转写语言无效".to_owned()));
        }
        for (label, value) in [
            ("分析模型", settings.analysis_model.trim()),
            ("嵌入模型", settings.embedding_model.trim()),
        ] {
            if value.is_empty() || value.len() > 120 {
                return Err(AppError::Invalid(format!("{label}无效")));
            }
        }
        let model_path = settings.whisper_model_path.trim();
        if !model_path.is_empty() && !Path::new(model_path).is_file() {
            return Err(AppError::Invalid(
                "选择的 Whisper 模型文件不存在".to_owned(),
            ));
        }
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let now = Utc::now().to_rfc3339();
        for (key, value) in [
            ("transcription_language", language),
            ("whisper_model_path", model_path),
            ("analysis_model", settings.analysis_model.trim()),
            ("embedding_model", settings.embedding_model.trim()),
        ] {
            transaction.execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                params![key, value, now],
            )?;
        }
        transaction.execute(
            "UPDATE knowledge_index_state SET status = 'stale', embedding_model = ?1, updated_at = ?2 WHERE embedding_model != ?1",
            params![settings.embedding_model.trim(), now],
        )?;
        transaction.commit()?;
        self.knowledge_settings()
    }

    pub fn external_ai_settings(&self, has_api_key: bool) -> AppResult<ExternalAiSettings> {
        Ok(ExternalAiSettings {
            enabled: self.setting("external_ai_enabled", "false")? == "true",
            base_url: self.setting("external_ai_base_url", "https://api.openai.com/v1")?,
            model: self.setting("external_ai_model", "gpt-4.1-mini")?,
            has_api_key,
            privacy_consent_at: match self.setting("external_ai_privacy_consent_at", "")? {
                value if value.is_empty() => None,
                value => Some(value),
            },
            transcription_provider: self.setting("cloud_transcription_provider", "none")?,
        })
    }

    pub fn update_external_ai_settings(
        &self,
        settings: &ExternalAiSettings,
        has_api_key: bool,
    ) -> AppResult<ExternalAiSettings> {
        let base_url = settings.base_url.trim().trim_end_matches('/');
        let model = settings.model.trim();
        if base_url.is_empty() || base_url.len() > 500 {
            return Err(AppError::Invalid("外部 AI Base URL 无效".to_owned()));
        }
        // 安全校验：解析真实 host 后判定（字符串前缀可被 userinfo/子域绕过，
        // 详见 memory.rs::validate_external_base_url）。
        crate::memory::validate_external_base_url(base_url)?;
        if model.is_empty() || model.len() > 160 {
            return Err(AppError::Invalid("外部 AI 模型名称无效".to_owned()));
        }
        if settings.enabled && settings.privacy_consent_at.is_none() {
            return Err(AppError::Invalid(
                "启用外部 AI 前必须确认文本发送说明".to_owned(),
            ));
        }
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let now = Utc::now().to_rfc3339();
        let consent = settings.privacy_consent_at.as_deref().unwrap_or("");
        for (key, value) in [
            (
                "external_ai_enabled",
                if settings.enabled { "true" } else { "false" },
            ),
            ("external_ai_base_url", base_url),
            ("external_ai_model", model),
            ("external_ai_privacy_consent_at", consent),
            ("cloud_transcription_provider", "none"),
        ] {
            transaction.execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                params![key, value, now],
            )?;
        }
        transaction.commit()?;
        self.external_ai_settings(has_api_key)
    }

    fn setting(&self, key: &str, default: &str) -> AppResult<String> {
        Ok(self
            .connect()?
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or_else(|| default.to_owned()))
    }

    pub fn list_analysis_templates(&self) -> AppResult<Vec<AnalysisTemplate>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, name, description, focus_instructions, custom_sections_json, is_builtin, created_at, updated_at FROM analysis_templates ORDER BY is_builtin DESC, name ASC",
        )?;
        let templates = statement
            .query_map([], Self::map_analysis_template)?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(templates)
    }

    pub fn get_analysis_template(&self, id: &str) -> AppResult<AnalysisTemplate> {
        self.connect()?
            .query_row(
                "SELECT id, name, description, focus_instructions, custom_sections_json, is_builtin, created_at, updated_at FROM analysis_templates WHERE id = ?1",
                params![id],
                Self::map_analysis_template,
            )
            .optional()?
            .ok_or_else(|| AppError::NotFound(format!("analysis template {id}")))
    }

    pub fn create_analysis_template(
        &self,
        name: &str,
        description: &str,
        focus_instructions: &str,
        sections: &[TemplateSection],
    ) -> AppResult<AnalysisTemplate> {
        Self::validate_template(name, focus_instructions, sections)?;
        let now = Utc::now().to_rfc3339();
        let id = Uuid::new_v4().to_string();
        let sections_json = serde_json::to_string(sections)
            .map_err(|error| AppError::Invalid(format!("模板栏目无效: {error}")))?;
        self.connect()?.execute(
            "INSERT INTO analysis_templates (id, name, description, focus_instructions, custom_sections_json, is_builtin, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?6)",
            params![id, name.trim(), description.trim(), focus_instructions.trim(), sections_json, now],
        )?;
        self.get_analysis_template(&id)
    }

    pub fn update_analysis_template(
        &self,
        id: &str,
        name: &str,
        description: &str,
        focus_instructions: &str,
        sections: &[TemplateSection],
    ) -> AppResult<AnalysisTemplate> {
        let current = self.get_analysis_template(id)?;
        if current.is_builtin {
            return Err(AppError::Invalid(
                "内置模板不可直接修改，请先复制".to_owned(),
            ));
        }
        Self::validate_template(name, focus_instructions, sections)?;
        let sections_json = serde_json::to_string(sections)
            .map_err(|error| AppError::Invalid(format!("模板栏目无效: {error}")))?;
        self.connect()?.execute(
            "UPDATE analysis_templates SET name = ?2, description = ?3, focus_instructions = ?4, custom_sections_json = ?5, updated_at = ?6 WHERE id = ?1",
            params![id, name.trim(), description.trim(), focus_instructions.trim(), sections_json, Utc::now().to_rfc3339()],
        )?;
        self.get_analysis_template(id)
    }

    pub fn delete_analysis_template(&self, id: &str) -> AppResult<()> {
        let current = self.get_analysis_template(id)?;
        if current.is_builtin {
            return Err(AppError::Invalid("内置模板不可删除".to_owned()));
        }
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE records SET analysis_template_id = 'builtin-standard' WHERE analysis_template_id = ?1",
            params![id],
        )?;
        transaction.execute("DELETE FROM analysis_templates WHERE id = ?1", params![id])?;
        transaction.commit()?;
        Ok(())
    }

    fn validate_template(
        name: &str,
        focus_instructions: &str,
        sections: &[TemplateSection],
    ) -> AppResult<()> {
        if name.trim().is_empty() || name.chars().count() > 40 {
            return Err(AppError::Invalid("模板名称应为 1 到 40 个字符".to_owned()));
        }
        if focus_instructions.trim().is_empty() || focus_instructions.chars().count() > 800 {
            return Err(AppError::Invalid("模板重点应为 1 到 800 个字符".to_owned()));
        }
        if sections.len() > 10 {
            return Err(AppError::Invalid("自定义栏目不能超过 10 个".to_owned()));
        }
        let mut keys = std::collections::HashSet::new();
        for section in sections {
            if section.key.trim().is_empty()
                || section.title.trim().is_empty()
                || !matches!(section.format.as_str(), "paragraph" | "list")
                || section.instruction.trim().is_empty()
                || !keys.insert(section.key.trim().to_owned())
            {
                return Err(AppError::Invalid("自定义栏目名称、格式或键无效".to_owned()));
            }
        }
        Ok(())
    }

    fn map_analysis_template(row: &Row<'_>) -> rusqlite::Result<AnalysisTemplate> {
        let sections_json: String = row.get(4)?;
        let custom_sections = serde_json::from_str::<Vec<TemplateSection>>(&sections_json)
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
        Ok(AnalysisTemplate {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            focus_instructions: row.get(3)?,
            custom_sections,
            is_builtin: row.get::<_, i64>(5)? != 0,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    }
}
