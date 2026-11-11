use super::LibraryRepository;
use crate::error::{AppError, AppResult};
use crate::provider_catalog;
use crate::types::{
    AnalysisTemplate, ExternalAiSettings, KnowledgeSettings, ProviderHealthSnapshot,
    TemplateSection,
};
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

    pub fn external_ai_settings(
        &self,
        has_api_key: bool,
        transcription_has_api_key: bool,
    ) -> AppResult<ExternalAiSettings> {
        let analysis_base_url = self.setting(
            "external_ai_base_url",
            "https://api.openai.com/v1",
        )?;
        let analysis_model = self.setting("external_ai_model", "gpt-4.1-mini")?;
        let analysis_provider = provider_catalog::normalize_provider_id(
            "analysis",
            &self.setting("external_ai_provider", "")?,
            &analysis_base_url,
        );
        let analysis = provider_catalog::resolve_config(
            "analysis",
            &analysis_provider,
            &analysis_base_url,
            &analysis_model,
        )
        .unwrap_or(provider_catalog::ResolvedProviderConfig {
            provider_id: analysis_provider,
            base_url: analysis_base_url,
            model: analysis_model,
        });
        let transcription_provider_stored =
            self.setting("cloud_transcription_provider", "openai-compatible")?;
        let transcription_base_url = self.setting(
            "cloud_transcription_base_url",
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
        )?;
        let transcription_model =
            self.setting("cloud_transcription_model", "qwen3-asr-flash")?;
        let transcription_provider = if transcription_provider_stored == "none" {
            "none".to_owned()
        } else {
            provider_catalog::normalize_provider_id(
                "transcription",
                &transcription_provider_stored,
                &transcription_base_url,
            )
        };
        let transcription = if transcription_provider == "none" {
            provider_catalog::ResolvedProviderConfig {
                provider_id: transcription_provider,
                base_url: transcription_base_url,
                model: transcription_model,
            }
        } else {
            provider_catalog::resolve_config(
                "transcription",
                &transcription_provider,
                &transcription_base_url,
                &transcription_model,
            )
            .unwrap_or(provider_catalog::ResolvedProviderConfig {
                provider_id: transcription_provider,
                base_url: transcription_base_url,
                model: transcription_model,
            })
        };
        let transcription_mode = self.setting(
            "cloud_transcription_mode",
            &self.setting("processing_mode", "local")?,
        )?;
        Ok(ExternalAiSettings {
            transcription_mode,
            enabled: self.setting("external_ai_enabled", "false")? == "true",
            analysis_provider: analysis.provider_id,
            base_url: analysis.base_url,
            model: analysis.model,
            has_api_key,
            privacy_consent_at: match self.setting("external_ai_privacy_consent_at", "")? {
                value if value.is_empty() => None,
                value => Some(value),
            },
            transcription_provider: transcription.provider_id,
            transcription_base_url: transcription.base_url,
            transcription_model: transcription.model,
            transcription_has_api_key,
            audio_upload_consent_at: match self
                .setting("cloud_audio_upload_consent_at", "")?
                .as_str()
            {
                "" => None,
                value => Some(value.to_owned()),
            },
        })
    }

    pub fn update_external_ai_settings(
        &self,
        settings: &ExternalAiSettings,
        has_api_key: bool,
        transcription_has_api_key: bool,
    ) -> AppResult<ExternalAiSettings> {
        if !matches!(settings.transcription_mode.as_str(), "local" | "external") {
            return Err(AppError::Invalid("录音处理通道无效".to_owned()));
        }
        let analysis_provider = provider_catalog::normalize_provider_id(
            "analysis",
            &settings.analysis_provider,
            &settings.base_url,
        );
        let analysis = provider_catalog::resolve_config(
            "analysis",
            &analysis_provider,
            &settings.base_url,
            &settings.model,
        )?;
        let base_url = analysis.base_url.as_str();
        // 安全校验：解析真实 host 后判定（字符串前缀可被 userinfo/子域绕过，
        // 详见 memory.rs::validate_external_base_url）。
        crate::memory::validate_external_base_url(base_url)?;
        if settings.enabled && settings.privacy_consent_at.is_none() {
            return Err(AppError::Invalid(
                "启用外部 AI 前必须确认文本发送说明".to_owned(),
            ));
        }
        let requested_transcription_provider = settings.transcription_provider.trim();
        let transcription_provider = if requested_transcription_provider == "none"
            || requested_transcription_provider.is_empty()
        {
            "none".to_owned()
        } else {
            provider_catalog::normalize_provider_id(
                "transcription",
                requested_transcription_provider,
                &settings.transcription_base_url,
            )
        };
        let transcription = if transcription_provider == "none" {
            provider_catalog::ResolvedProviderConfig {
                provider_id: "none".to_owned(),
                base_url: settings
                    .transcription_base_url
                    .trim()
                    .trim_end_matches('/')
                    .to_owned(),
                model: settings.transcription_model.trim().to_owned(),
            }
        } else {
            provider_catalog::resolve_config(
                "transcription",
                &transcription_provider,
                &settings.transcription_base_url,
                &settings.transcription_model,
            )?
        };
        if transcription.provider_id != "none" {
            crate::memory::validate_external_base_url(&transcription.base_url)?;
        }
        let previous = self.external_ai_settings(false, false)?;
        let analysis_target_changed = previous.analysis_provider != analysis.provider_id
            || previous.base_url != analysis.base_url
            || previous.model != analysis.model;
        let transcription_target_changed =
            previous.transcription_provider != transcription.provider_id
                || previous.transcription_base_url != transcription.base_url
                || previous.transcription_model != transcription.model;
        let enabled = if analysis_target_changed {
            false
        } else {
            settings.enabled
        };
        let privacy_consent = if analysis_target_changed {
            ""
        } else {
            settings.privacy_consent_at.as_deref().unwrap_or("")
        };
        let audio_consent = if transcription_target_changed {
            ""
        } else {
            settings.audio_upload_consent_at.as_deref().unwrap_or("")
        };
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let now = Utc::now().to_rfc3339();
        for (key, value) in [
            (
                "cloud_transcription_mode",
                settings.transcription_mode.as_str(),
            ),
            (
                "external_ai_enabled",
                if enabled { "true" } else { "false" },
            ),
            ("external_ai_provider", analysis.provider_id.as_str()),
            ("external_ai_base_url", analysis.base_url.as_str()),
            ("external_ai_model", analysis.model.as_str()),
            ("external_ai_privacy_consent_at", privacy_consent),
            (
                "cloud_transcription_provider",
                transcription.provider_id.as_str(),
            ),
            (
                "cloud_transcription_base_url",
                transcription.base_url.as_str(),
            ),
            (
                "cloud_transcription_model",
                transcription.model.as_str(),
            ),
            ("cloud_audio_upload_consent_at", audio_consent),
        ] {
            transaction.execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                params![key, value, now],
            )?;
        }
        transaction.commit()?;
        self.external_ai_settings(has_api_key, transcription_has_api_key)
    }

    // ---- 第三方连接健康快照与凭据修订号（Phase 0）----

    fn revision_key(target: &str) -> AppResult<String> {
        match target {
            "analysis" | "transcription" => {
                Ok(format!("external_ai_credential_revision.{target}"))
            }
            other => Err(AppError::Invalid(format!("凭据目标无效：{other}"))),
        }
    }

    /// 当前凭据修订号（无凭据历史时为 0）。凭据任何写入都会使它递增，
    /// 旧的健康快照随即失效。
    pub fn credential_revision(&self, target: &str) -> AppResult<i64> {
        let key = Self::revision_key(target)?;
        self.setting(&key, "0")?
            .trim()
            .parse::<i64>()
            .map_err(|_| AppError::Invalid("凭据修订号损坏".to_owned()))
    }

    /// 递增凭据修订号并返回新值。必须与凭据写入在同一次提交语义内调用。
    pub fn bump_credential_revision(&self, target: &str) -> AppResult<i64> {
        let key = Self::revision_key(target)?;
        let next = self.credential_revision(target)? + 1;
        let connection = self.connect()?;
        connection.execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, next.to_string(), Utc::now().to_rfc3339()],
        )?;
        Ok(next)
    }

    /// 读取某槽位的健康快照（最后两次验证中最新的一次，含 failed）。
    pub fn provider_health(&self, target: &str) -> AppResult<Option<ProviderHealthSnapshot>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT target, provider_id, base_url, model, credential_revision, catalog_version, stage, status, error_code, verified_at FROM provider_health WHERE target = ?1",
        )?;
        let mut rows = statement.query(params![target])?;
        Ok(match rows.next()? {
            Some(row) => Some(ProviderHealthSnapshot {
                target: row.get(0)?,
                provider_id: row.get(1)?,
                base_url: row.get(2)?,
                model: row.get(3)?,
                credential_revision: row.get(4)?,
                catalog_version: row.get(5)?,
                stage: row.get(6)?,
                status: row.get(7)?,
                error_code: row.get(8)?,
                verified_at: row.get(9)?,
            }),
            None => None,
        })
    }

    /// 写入健康快照（覆盖同一槽位旧值）。
    pub fn save_provider_health(&self, snapshot: &ProviderHealthSnapshot) -> AppResult<()> {
        let connection = self.connect()?;
        connection.execute(
            "INSERT INTO provider_health (target, provider_id, base_url, model, credential_revision, catalog_version, stage, status, error_code, verified_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) ON CONFLICT(target) DO UPDATE SET provider_id = excluded.provider_id, base_url = excluded.base_url, model = excluded.model, credential_revision = excluded.credential_revision, catalog_version = excluded.catalog_version, stage = excluded.stage, status = excluded.status, error_code = excluded.error_code, verified_at = excluded.verified_at",
            params![
                snapshot.target,
                snapshot.provider_id,
                snapshot.base_url,
                snapshot.model,
                snapshot.credential_revision,
                snapshot.catalog_version,
                snapshot.stage,
                snapshot.status,
                snapshot.error_code,
                snapshot.verified_at,
            ],
        )?;
        Ok(())
    }

    /// 「保存并验证」通过后的提交：配置 + 凭据修订号 + 健康快照在单个
    /// SQLite 事务内落盘。与 `update_external_ai_settings` 的区别：
    /// 服务商/地址/模型变更**不**重置同意与启用——变更已经过真实验证，
    /// 重置反而会把用户放回「未启用」的半状态。规范化与安全校验同规。
    pub fn commit_external_provider(
        &self,
        settings: &ExternalAiSettings,
        has_api_key: bool,
        transcription_has_api_key: bool,
        bump_revision_for: Option<&str>,
        health: Option<&ProviderHealthSnapshot>,
    ) -> AppResult<ExternalAiSettings> {
        if !matches!(settings.transcription_mode.as_str(), "local" | "external") {
            return Err(AppError::Invalid("录音处理通道无效".to_owned()));
        }
        let analysis_provider = provider_catalog::normalize_provider_id(
            "analysis",
            &settings.analysis_provider,
            &settings.base_url,
        );
        let analysis = provider_catalog::resolve_config(
            "analysis",
            &analysis_provider,
            &settings.base_url,
            &settings.model,
        )?;
        crate::memory::validate_external_base_url(&analysis.base_url)?;
        let transcription_provider = provider_catalog::normalize_provider_id(
            "transcription",
            &settings.transcription_provider,
            &settings.transcription_base_url,
        );
        let transcription = provider_catalog::resolve_config(
            "transcription",
            &transcription_provider,
            &settings.transcription_base_url,
            &settings.transcription_model,
        )?;
        crate::memory::validate_external_base_url(&transcription.base_url)?;

        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let now = Utc::now().to_rfc3339();
        for (key, value) in [
            (
                "cloud_transcription_mode",
                settings.transcription_mode.as_str(),
            ),
            (
                "external_ai_enabled",
                if settings.enabled { "true" } else { "false" },
            ),
            ("external_ai_provider", analysis.provider_id.as_str()),
            ("external_ai_base_url", analysis.base_url.as_str()),
            ("external_ai_model", analysis.model.as_str()),
            (
                "external_ai_privacy_consent_at",
                settings.privacy_consent_at.as_deref().unwrap_or(""),
            ),
            (
                "cloud_transcription_provider",
                transcription.provider_id.as_str(),
            ),
            (
                "cloud_transcription_base_url",
                transcription.base_url.as_str(),
            ),
            (
                "cloud_transcription_model",
                transcription.model.as_str(),
            ),
            (
                "cloud_audio_upload_consent_at",
                settings.audio_upload_consent_at.as_deref().unwrap_or(""),
            ),
        ] {
            transaction.execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                params![key, value, now],
            )?;
        }
        if let Some(target) = bump_revision_for {
            let key = Self::revision_key(target)?;
            let current: i64 = transaction
                .query_row(
                    "SELECT value FROM app_settings WHERE key = ?1",
                    params![key],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .and_then(|value| value.trim().parse::<i64>().ok())
                .unwrap_or(0);
            let next = current + 1;
            transaction.execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                params![key, next.to_string(), now],
            )?;
        }
        if let Some(health) = health {
            transaction.execute(
                "INSERT INTO provider_health (target, provider_id, base_url, model, credential_revision, catalog_version, stage, status, error_code, verified_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) ON CONFLICT(target) DO UPDATE SET provider_id = excluded.provider_id, base_url = excluded.base_url, model = excluded.model, credential_revision = excluded.credential_revision, catalog_version = excluded.catalog_version, stage = excluded.stage, status = excluded.status, error_code = excluded.error_code, verified_at = excluded.verified_at",
                params![
                    health.target,
                    health.provider_id,
                    health.base_url,
                    health.model,
                    health.credential_revision,
                    health.catalog_version,
                    health.stage,
                    health.status,
                    health.error_code,
                    health.verified_at,
                ],
            )?;
        }
        transaction.commit()?;
        self.external_ai_settings(has_api_key, transcription_has_api_key)
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

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_repository() -> (LibraryRepository, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("external-settings-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        (
            LibraryRepository::new(root.join("library.db")).unwrap(),
            root,
        )
    }

    #[test]
    fn credential_revision_is_monotonic_per_target() {
        let (repository, root) = temp_repository();
        assert_eq!(repository.credential_revision("analysis").unwrap(), 0);
        assert_eq!(repository.bump_credential_revision("analysis").unwrap(), 1);
        assert_eq!(repository.bump_credential_revision("analysis").unwrap(), 2);
        // 转写槽位独立计数。
        assert_eq!(repository.credential_revision("transcription").unwrap(), 0);
        assert_eq!(repository.bump_credential_revision("transcription").unwrap(), 1);
        assert_eq!(repository.credential_revision("analysis").unwrap(), 2);
        // 非法目标拒绝。
        assert!(repository.bump_credential_revision("other").is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn provider_health_snapshot_overwrites_and_fails_revision_match() {
        let (repository, root) = temp_repository();
        assert!(repository.provider_health("analysis").unwrap().is_none());

        let mut snapshot = crate::types::ProviderHealthSnapshot {
            target: "analysis".to_owned(),
            provider_id: "dashscope".to_owned(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_owned(),
            model: "qwen-turbo".to_owned(),
            credential_revision: 1,
            catalog_version: 1,
            stage: "ok".to_owned(),
            status: "passed".to_owned(),
            error_code: None,
            verified_at: "2026-11-11T00:00:00Z".to_owned(),
        };
        repository.save_provider_health(&snapshot).unwrap();
        let saved = repository.provider_health("analysis").unwrap().unwrap();
        assert_eq!(saved, snapshot);

        // 失败结果也会覆盖（用户重启后能看到「验证失败」）。
        snapshot.status = "failed".to_owned();
        snapshot.error_code = Some("model_not_found".to_owned());
        snapshot.credential_revision = 2;
        repository.save_provider_health(&snapshot).unwrap();
        let failed = repository.provider_health("analysis").unwrap().unwrap();
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.error_code.as_deref(), Some("model_not_found"));
        assert_eq!(failed.credential_revision, 2);

        // 转写槽位互不干扰。
        assert!(repository.provider_health("transcription").unwrap().is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn commit_external_provider_writes_health_revision_matching_the_bump() {
        let (repository, root) = temp_repository();
        let mut settings = repository.external_ai_settings(false, false).unwrap();
        settings.analysis_provider = "dashscope".to_owned();
        settings.model = "qwen-turbo".to_owned();
        settings.privacy_consent_at = Some("2026-11-11T00:00:00Z".to_owned());
        settings.enabled = true;
        // 快照的修订号必须与事务内 bump 后的真实值一致（当前 0 + 1），
        // 否则「保存并验证」成功后指纹永不相配、「已验证」不可达。
        let next_revision = repository.credential_revision("analysis").unwrap() + 1;
        let health = crate::types::ProviderHealthSnapshot {
            target: "analysis".to_owned(),
            provider_id: "dashscope".to_owned(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_owned(),
            model: "qwen-turbo".to_owned(),
            credential_revision: next_revision,
            catalog_version: 1,
            stage: "ok".to_owned(),
            status: "passed".to_owned(),
            error_code: None,
            verified_at: "2026-11-11T00:00:00Z".to_owned(),
        };
        repository
            .commit_external_provider(&settings, true, false, Some("analysis"), Some(&health))
            .unwrap();

        let stored = repository.provider_health("analysis").unwrap().unwrap();
        let revision = repository.credential_revision("analysis").unwrap();
        assert_eq!(revision, 1, "事务内 bump 只发生一次");
        assert_eq!(
            stored.credential_revision, revision,
            "快照修订号必须与凭据修订号一致，否则已验证态不可达"
        );
        assert_eq!(stored.status, "passed");
        assert_eq!(stored.provider_id, "dashscope");

        // 指纹全匹配：revision/catalog/provider/base_url/model 一致即「已验证」依据成立。
        let reloaded = repository.external_ai_settings(true, false).unwrap();
        assert_eq!(reloaded.analysis_provider, "dashscope");
        assert_eq!(reloaded.model, "qwen-turbo");
        assert_eq!(reloaded.privacy_consent_at.as_deref(), Some("2026-11-11T00:00:00Z"));
        assert!(reloaded.enabled);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn processing_mode_and_independent_asr_settings_persist() {
        let (repository, root) = temp_repository();
        let defaults = repository.external_ai_settings(false, false).unwrap();
        assert_eq!(defaults.transcription_mode, "local");
        assert!(!defaults.transcription_has_api_key);

        let mut next = defaults;
        next.transcription_mode = "external".to_owned();
        next.transcription_provider = "custom".to_owned();
        next.transcription_base_url = "https://api.example.com/v1".to_owned();
        next.transcription_model = "asr-model".to_owned();
        repository
            .update_external_ai_settings(&next, false, true)
            .unwrap();
        let mut next = repository.external_ai_settings(false, true).unwrap();
        next.audio_upload_consent_at = Some("2026-09-25T00:00:00Z".to_owned());
        repository
            .update_external_ai_settings(&next, false, true)
            .unwrap();

        let saved = repository.external_ai_settings(false, true).unwrap();
        assert_eq!(saved.transcription_mode, "external");
        assert_eq!(saved.transcription_provider, "custom");
        assert_eq!(saved.transcription_base_url, "https://api.example.com/v1");
        assert_eq!(saved.transcription_model, "asr-model");
        assert!(saved.transcription_has_api_key);
        assert_eq!(
            saved.audio_upload_consent_at.as_deref(),
            Some("2026-09-25T00:00:00Z")
        );

        let mut invalid = saved;
        invalid.transcription_mode = "unknown".to_owned();
        assert!(repository
            .update_external_ai_settings(&invalid, false, true)
            .is_err());
        drop(repository);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn external_mode_persists_before_asr_provider_is_selected() {
        let (repository, root) = temp_repository();
        let mut next = repository.external_ai_settings(false, false).unwrap();
        next.transcription_mode = "external".to_owned();
        next.transcription_provider = "none".to_owned();

        let saved = repository
            .update_external_ai_settings(&next, false, false)
            .unwrap();

        assert_eq!(saved.transcription_mode, "external");
        assert_eq!(saved.transcription_provider, "none");
        drop(repository);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_processing_mode_is_only_a_read_fallback() {
        let (repository, root) = temp_repository();
        repository
            .set_setting_value("processing_mode", "external")
            .unwrap();
        let migrated = repository.external_ai_settings(false, false).unwrap();
        assert_eq!(migrated.transcription_mode, "external");

        let mut next = migrated;
        next.transcription_mode = "local".to_owned();
        repository
            .update_external_ai_settings(&next, false, false)
            .unwrap();
        repository
            .set_setting_value("processing_mode", "external")
            .unwrap();
        assert_eq!(
            repository
                .external_ai_settings(false, false)
                .unwrap()
                .transcription_mode,
            "local",
            "新字段写入后不得继续被旧 processing_mode 覆盖"
        );
        drop(repository);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn changing_analysis_target_clears_consent_and_disables_external_ai() {
        let (repository, root) = temp_repository();
        let mut next = repository.external_ai_settings(false, false).unwrap();
        next.enabled = true;
        next.privacy_consent_at = Some("2026-09-25T00:00:00Z".to_owned());
        let saved = repository
            .update_external_ai_settings(&next, true, false)
            .unwrap();
        assert!(saved.enabled);
        assert!(saved.privacy_consent_at.is_some());

        let mut changed = saved;
        changed.analysis_provider = "custom".to_owned();
        changed.base_url = "https://relay.example/v1".to_owned();
        changed.model = "analysis-model".to_owned();
        let saved = repository
            .update_external_ai_settings(&changed, true, false)
            .unwrap();
        assert!(!saved.enabled);
        assert!(saved.privacy_consent_at.is_none());
        drop(repository);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn changing_transcription_target_clears_only_audio_consent() {
        let (repository, root) = temp_repository();
        let mut next = repository.external_ai_settings(false, false).unwrap();
        next.enabled = true;
        next.privacy_consent_at = Some("2026-09-25T00:00:00Z".to_owned());
        next.transcription_mode = "external".to_owned();
        next.transcription_provider = "custom".to_owned();
        next.transcription_base_url = "https://asr.example/v1".to_owned();
        next.transcription_model = "asr-model".to_owned();
        repository
            .update_external_ai_settings(&next, true, true)
            .unwrap();
        let mut next = repository.external_ai_settings(true, true).unwrap();
        next.audio_upload_consent_at = Some("2026-09-25T00:00:00Z".to_owned());
        let saved = repository
            .update_external_ai_settings(&next, true, true)
            .unwrap();
        assert!(saved.audio_upload_consent_at.is_some());

        let mut changed = saved;
        changed.transcription_provider = "zhipu".to_owned();
        changed.transcription_base_url = "https://open.bigmodel.cn/api/paas/v4".to_owned();
        changed.transcription_model = "glm-asr-2512".to_owned();
        let saved = repository
            .update_external_ai_settings(&changed, true, true)
            .unwrap();
        assert!(saved.audio_upload_consent_at.is_none());
        assert!(saved.privacy_consent_at.is_some());
        assert!(saved.enabled);
        drop(repository);
        let _ = std::fs::remove_dir_all(root);
    }
}
