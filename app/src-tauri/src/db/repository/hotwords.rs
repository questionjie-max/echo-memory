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
            "INSERT INTO hotwords (id, term, note, created_at) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(term) DO UPDATE SET note = \
             CASE WHEN excluded.note = '' THEN hotwords.note ELSE excluded.note END",
            params![hotword.id, hotword.term, hotword.note, hotword.created_at],
        )?;
        Ok(hotword)
    }

    pub fn remove_hotword(&self, id: &str) -> AppResult<()> {
        self.connect()?
            .execute("DELETE FROM hotwords WHERE id = ?1", params![id])?;
        Ok(())
    }

    /* ---------------- 术语变体映射（v1.1 块7） ---------------- */

    /// 登记一个识别变体 → 规范词映射。变体全库唯一（一个错写只归一个规范词）；
    /// 变体等于规范词本身时是空操作（不登记自指映射）；变体至少 2 个字符
    /// （搜索扩展按子串匹配，单字变体会对大量查询误扩展）。
    pub fn add_term_variant(&self, term: &str, variant: &str) -> AppResult<()> {
        let term = term.trim();
        let variant = variant.trim();
        if term.is_empty() || variant.is_empty() || term == variant {
            return Ok(());
        }
        if term.chars().count() > 40
            || variant.chars().count() > 40
            || variant.chars().count() < 2
        {
            return Err(crate::error::AppError::Invalid("术语变体无效（需 2-40 字）".to_owned()));
        }
        self.connect()?.execute(
            "INSERT INTO term_variants (id, term, variant, created_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(variant) DO UPDATE SET term = excluded.term",
            params![Uuid::new_v4().to_string(), term, variant, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn remove_term_variant(&self, variant: &str) -> AppResult<()> {
        self.connect()?
            .execute("DELETE FROM term_variants WHERE variant = ?1", params![variant])?;
        Ok(())
    }

    pub fn list_term_variants(&self, term: &str) -> AppResult<Vec<String>> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare("SELECT variant FROM term_variants WHERE term = ?1 ORDER BY created_at")?;
        let rows = statement
            .query_map(params![term], |row| row.get(0))?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    /// 纪要术语表 → 热词库 + 变体映射的联动沉淀（块7）。
    /// 术语进热词库（后续转写 initial_prompt 与 AI 校对都会带上它），
    /// 变体进映射表（错写搜索能命中规范写法的原文）。
    pub fn sync_minutes_glossary(&self, glossary: &[crate::minutes::GlossaryEntry]) -> AppResult<()> {
        for entry in glossary {
            if entry.term.trim().is_empty() {
                continue;
            }
            self.add_hotword(&entry.term, "")?;
            for variant in &entry.variants {
                self.add_term_variant(&entry.term, variant)?;
            }
        }
        Ok(())
    }

    /// 搜索前的变体扩展：查询里包含某个已登记变体（子串匹配，变体至少
    /// 2 个字符防误扩展）时，返回对应规范写法与同词其它变体。
    /// 例：查「张叁负责」命中变体「张叁」→ 扩展出「张三」。
    pub fn term_variant_expansions(&self, query: &str) -> AppResult<Vec<String>> {
        let query = query.trim();
        if query.chars().count() < 2 {
            return Ok(Vec::new());
        }
        let connection = self.connect()?;
        let mut statement = connection
            .prepare("SELECT term, variant FROM term_variants WHERE instr(?1, variant) > 0")?;
        let matches = statement
            .query_map(params![query], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()
            .map_err(|error| crate::error::AppError::Invalid(format!("变体查询失败：{error}")))?;
        // 扩展去重且不回带查询里已有的写法。
        let mut expansions: Vec<String> = Vec::new();
        let mut push = |candidate: String| {
            if candidate != query && !expansions.contains(&candidate) {
                expansions.push(candidate);
            }
        };
        for (term, _variant) in &matches {
            push(term.clone());
            for variant in self.list_term_variants(term)? {
                push(variant);
            }
        }
        Ok(expansions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_library() -> crate::library::ManagedLibrary {
        let root = std::env::temp_dir().join(format!("echo-hotwords-{}", Uuid::new_v4()));
        crate::library::ManagedLibrary::open(root).unwrap()
    }

    #[test]
    fn variants_roundtrip_and_reject_self_mapping() {
        let library = test_library();
        let repository = library.repository();
        repository.add_term_variant("张三", "张叁").unwrap();
        repository.add_term_variant("张三", "Zhang San").unwrap();
        assert_eq!(
            repository.list_term_variants("张三").unwrap(),
            vec!["张叁".to_owned(), "Zhang San".to_owned()]
        );
        // 自指映射：空操作。
        repository.add_term_variant("张三", "张三").unwrap();
        assert_eq!(repository.list_term_variants("张三").unwrap().len(), 2);
        // 变体改归属：后来者接管（UNIQUE(variant) upsert）；未被接管的
        // "Zhang San" 仍属张三。
        repository.add_term_variant("李四", "张叁").unwrap();
        assert_eq!(
            repository.list_term_variants("张三").unwrap(),
            vec!["Zhang San".to_owned()]
        );
        assert_eq!(repository.list_term_variants("李四").unwrap(), vec!["张叁"]);
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn expansions_return_term_and_sibling_variants() {
        let library = test_library();
        let repository = library.repository();
        repository.add_term_variant("张三", "张叁").unwrap();
        repository.add_term_variant("张三", "Zhang San").unwrap();
        let expansions = repository.term_variant_expansions("张叁").unwrap();
        assert_eq!(
            expansions,
            vec!["张三".to_owned(), "Zhang San".to_owned()]
        );
        // 未登记的查询词：无扩展。
        assert!(repository.term_variant_expansions("王五").unwrap().is_empty());
        // 变体换归属后按新词扩展。
        repository.add_term_variant("李四", "张叁").unwrap();
        let expansions = repository.term_variant_expansions("张叁").unwrap();
        assert_eq!(expansions.first(), Some(&"李四".to_owned()));
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn glossary_sync_feeds_hotwords_and_variants() {
        let library = test_library();
        let repository = library.repository();
        repository
            .sync_minutes_glossary(&[crate::minutes::GlossaryEntry {
                term: "回声记忆".to_owned(),
                variants: vec!["回声记意".to_owned(), "EchoMemory".to_owned()],
            }])
            .unwrap();
        let hotwords = repository.list_hotwords().unwrap();
        assert_eq!(hotwords.len(), 1);
        assert_eq!(hotwords[0].term, "回声记忆");
        assert_eq!(
            repository.list_term_variants("回声记忆").unwrap().len(),
            2
        );
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn glossary_sync_never_overwrites_user_note_with_empty() {
        // 块7 审核 P1：纪要每次生成都重跑 sync，空 note 不得抹掉用户已写的备注。
        let library = test_library();
        let repository = library.repository();
        let hotword = repository.add_hotword("张三", "产品负责人").unwrap();
        repository
            .sync_minutes_glossary(&[crate::minutes::GlossaryEntry {
                term: "张三".to_owned(),
                variants: vec!["张叁".to_owned()],
            }])
            .unwrap();
        let hotwords = repository.list_hotwords().unwrap();
        assert_eq!(hotwords.len(), 1);
        assert_eq!(hotwords[0].id, hotword.id, "不应新建重复热词");
        assert_eq!(hotwords[0].note, "产品负责人", "空 note 不得覆盖用户备注");
        // 带 note 的写入仍然更新备注。
        repository.add_hotword("张三", "新的备注").unwrap();
        assert_eq!(repository.list_hotwords().unwrap()[0].note, "新的备注");
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn single_character_variant_is_rejected() {
        let library = test_library();
        let repository = library.repository();
        assert!(repository.add_term_variant("张三", "张").is_err());
        assert!(repository.list_term_variants("张三").unwrap().is_empty());
        let _ = std::fs::remove_dir_all(library.root());
    }
}
