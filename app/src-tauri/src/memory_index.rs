//! 记忆检索索引：把用户确认过的记忆条目投影成 memory_chunks，
//! 供问知识库与记忆条目并行召回。
//!
//! 三条硬规则：
//! 1. 只收用户明确「确认」的条目——rejected 绝不入库，未确认暂不入库；
//! 2. 只投影每个 (view_kind, scope, range) 的最新 completed 快照；
//! 3. 条目反馈跨快照继承：同一稳定内容 ID 取最近一次裁决（后写覆盖先写）。

use crate::error::{AppError, AppResult};
use crate::library::ManagedLibrary;
use crate::types::{EvolutionItem, MemoryChunkInput, MemoryNode, MemorySourceReference};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// 节点类型白名单：只收观点与决定，事件/人物/主题节点信息密度不足以单独成块。
const INDEXABLE_NODE_TYPES: [&str; 2] = ["viewpoint", "decision"];

const CHANGE_TYPE_LABELS: [(&str, &str); 6] = [
    ("added", "新增"),
    ("supplemented", "补充"),
    ("revised", "修正"),
    ("overturned", "推翻"),
    ("merged", "合并"),
    ("validated", "验证"),
];

fn change_type_label(raw: &str) -> &str {
    let normalized = raw.trim().to_lowercase();
    CHANGE_TYPE_LABELS
        .iter()
        .find(|(key, _)| *key == normalized)
        .map(|(_, label)| *label)
        .unwrap_or("变化")
}

struct MemoryChunkDraft {
    snapshot_id: String,
    view_kind: String,
    item_id: String,
    item_type: String,
    topic: String,
    before_text: String,
    after_text: String,
    body: String,
    occurred_at: String,
    sources: Vec<MemorySourceReference>,
}

/// 全量重建记忆块索引。个人量级（确认条目通常数百以内）一次性批量嵌入即可，
/// 全量替换让「快照重生成后旧块不残留」由事务天然成立。
pub fn rebuild_memory_index(library: &ManagedLibrary) -> AppResult<()> {
    let repository = library.repository();
    let settings = repository.knowledge_settings()?;
    let model = settings.embedding_model.clone();
    let mut status = repository.get_knowledge_index_status("memory", &model)?;
    if status.status == "indexing" {
        return Ok(());
    }
    status.status = "indexing".to_owned();
    status.last_error = None;
    status.updated_at = chrono::Utc::now().to_rfc3339();
    repository.save_knowledge_index_status(&status)?;

    let result = rebuild_inner(library, &model);
    match result {
        Ok(count) => {
            let mut status = repository.get_knowledge_index_status("memory", &model)?;
            status.status = "completed".to_owned();
            status.chunk_count = count as i64;
            status.last_error = None;
            status.updated_at = chrono::Utc::now().to_rfc3339();
            repository.save_knowledge_index_status(&status)?;
            Ok(())
        }
        Err(error) => {
            let mut status = repository.get_knowledge_index_status("memory", &model)?;
            status.status = "failed".to_owned();
            status.last_error = Some(error.to_string());
            status.updated_at = chrono::Utc::now().to_rfc3339();
            repository.save_knowledge_index_status(&status)?;
            Err(error)
        }
    }
}

fn rebuild_inner(library: &ManagedLibrary, model: &str) -> AppResult<usize> {
    let repository = library.repository();
    let snapshots = repository.latest_completed_memory_snapshots()?;
    let decisions = repository.latest_memory_feedback_decisions()?;
    let mut drafts: Vec<MemoryChunkDraft> = Vec::new();
    let mut seen_items: HashSet<String> = HashSet::new();
    for (snapshot_id, view_kind, result_json) in &snapshots {
        let Ok(snapshot) = serde_json::from_str::<crate::types::MemorySnapshotResult>(result_json)
        else {
            continue;
        };
        for item in &snapshot.evolution_items {
            if !confirmed(&item.id, &decisions) || !seen_items.insert(item.id.clone()) {
                continue;
            }
            if let Some(draft) = evolution_draft(snapshot_id, view_kind, item) {
                drafts.push(draft);
            }
        }
        for node in &snapshot.nodes {
            if !INDEXABLE_NODE_TYPES.contains(&node.node_type.as_str()) {
                continue;
            }
            if !confirmed(&node.id, &decisions) || !seen_items.insert(node.id.clone()) {
                continue;
            }
            if let Some(draft) = node_draft(snapshot_id, view_kind, node) {
                drafts.push(draft);
            }
        }
    }

    // 没有可收录条目也要落一次空替换：把旧块清干净，计数归零。
    // 空投影不需求嵌入模型：Ollama 没开时清空索引也应成功。
    if !drafts.is_empty() {
        crate::knowledge::ensure_model_installed(model)?;
    }
    let texts = drafts
        .iter()
        .map(|draft| draft.body.clone())
        .collect::<Vec<_>>();
    let embeddings = crate::knowledge::embed_texts(model, &texts)?;
    if embeddings.len() != drafts.len() {
        return Err(AppError::Analysis(
            "嵌入模型返回的向量数量不正确".to_owned(),
        ));
    }
    let inputs = drafts
        .into_iter()
        .zip(embeddings)
        .map(|(draft, embedding)| {
            let content_hash = format!("{:x}", Sha256::digest(draft.body.as_bytes()));
            MemoryChunkInput {
                id: Uuid::new_v4().to_string(),
                snapshot_id: draft.snapshot_id,
                view_kind: draft.view_kind,
                item_id: draft.item_id,
                item_type: draft.item_type,
                topic: draft.topic,
                before_text: draft.before_text,
                after_text: draft.after_text,
                body: draft.body,
                occurred_at: draft.occurred_at,
                sources: draft.sources,
                content_hash,
                embedding_model: model.to_owned(),
                embedding,
            }
        })
        .collect::<Vec<_>>();
    let count = inputs.len();
    repository.replace_all_memory_chunks(&inputs)?;
    Ok(count)
}

fn confirmed(item_id: &str, decisions: &HashMap<String, String>) -> bool {
    decisions.get(item_id).map(String::as_str) == Some("confirmed")
}

fn evolution_draft(
    snapshot_id: &str,
    view_kind: &str,
    item: &EvolutionItem,
) -> Option<MemoryChunkDraft> {
    // 实义字数按条目字段直接统计，避免把模板骨架（标签、类型、日期）算成内容。
    let content_chars = item.topic.chars().count()
        + item.before_text.chars().count()
        + item.after_text.chars().count()
        + item.reason.chars().count();
    if content_chars < 10 {
        return None;
    }
    let mut lines = Vec::new();
    if !item.topic.trim().is_empty() {
        lines.push(format!("主题：{}", item.topic.trim()));
    }
    if !item.before_text.trim().is_empty() {
        lines.push(format!("此前：{}", item.before_text.trim()));
    }
    if !item.after_text.trim().is_empty() {
        lines.push(format!("现在：{}", item.after_text.trim()));
    }
    if !item.reason.trim().is_empty() {
        lines.push(format!("变化原因：{}", item.reason.trim()));
    }
    lines.push(format!(
        "（{} · {}）",
        change_type_label(&item.change_type),
        occurred_date(&item.occurred_at)
    ));
    let body = lines.join("\n");
    Some(MemoryChunkDraft {
        snapshot_id: snapshot_id.to_owned(),
        view_kind: view_kind.to_owned(),
        item_id: item.id.clone(),
        item_type: "evolution".to_owned(),
        topic: item.topic.trim().to_owned(),
        before_text: item.before_text.trim().to_owned(),
        after_text: item.after_text.trim().to_owned(),
        body,
        occurred_at: item.occurred_at.clone(),
        sources: item.sources.clone(),
    })
}

fn node_draft(snapshot_id: &str, view_kind: &str, node: &MemoryNode) -> Option<MemoryChunkDraft> {
    let heading = match node.node_type.as_str() {
        "viewpoint" => "观点",
        "decision" => "决定",
        _ => return None,
    };
    if node.label.chars().count() + node.summary.chars().count() < 10 {
        return None;
    }
    let mut lines = Vec::new();
    if !node.label.trim().is_empty() {
        lines.push(format!("{heading}：{}", node.label.trim()));
    }
    if !node.summary.trim().is_empty() {
        lines.push(node.summary.trim().to_owned());
    }
    let body = lines.join("\n");
    Some(MemoryChunkDraft {
        snapshot_id: snapshot_id.to_owned(),
        view_kind: view_kind.to_owned(),
        item_id: node.id.clone(),
        item_type: node.node_type.clone(),
        topic: node.label.trim().to_owned(),
        before_text: String::new(),
        after_text: String::new(),
        body,
        occurred_at: String::new(),
        sources: node.sources.clone(),
    })
}

fn occurred_date(occurred_at: &str) -> String {
    occurred_at.chars().take(10).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evolution(topic: &str, before: &str, after: &str, change: &str) -> EvolutionItem {
        serde_json::from_value(serde_json::json!({
            "id": "e1",
            "topic": topic,
            "changeType": change,
            "beforeText": before,
            "afterText": after,
            "reason": "新的证据改变了判断",
            "occurredAt": "2026-09-01T00:00:00Z",
            "inferred": true,
            "confidence": 0.8,
            "sources": []
        }))
        .unwrap()
    }

    #[test]
    fn evolution_body_includes_confirmed_sections_and_chinese_label() {
        let item = evolution("定价", "按次收费", "订阅制", "revised");
        let draft = evolution_draft("snap1", "evolution", &item).unwrap();
        assert!(draft.body.contains("主题：定价"));
        assert!(draft.body.contains("此前：按次收费"));
        assert!(draft.body.contains("现在：订阅制"));
        assert!(draft.body.contains("修正"));
        assert!(draft.body.contains("2026-09-01"));
        assert_eq!(draft.item_type, "evolution");
    }

    #[test]
    fn skeleton_only_items_are_dropped() {
        let item = evolution("", "", "", "added");
        assert!(evolution_draft("snap1", "evolution", &item).is_none());
    }

    #[test]
    fn node_body_only_accepts_viewpoint_and_decision() {
        let node = serde_json::from_value::<MemoryNode>(serde_json::json!({
            "id": "n1",
            "nodeType": "theme",
            "label": "某主题",
            "summary": "一些说明",
            "inferred": true,
            "confidence": 0.7,
            "sources": []
        }))
        .unwrap();
        assert!(node_draft("snap1", "map", &node).is_none());

        let viewpoint = serde_json::from_value::<MemoryNode>(serde_json::json!({
            "id": "n2",
            "nodeType": "viewpoint",
            "label": "本地优先不可妥协",
            "summary": "云依赖必须显式同意。",
            "inferred": true,
            "confidence": 0.9,
            "sources": []
        }))
        .unwrap();
        let draft = node_draft("snap1", "map", &viewpoint).unwrap();
        assert!(draft.body.contains("观点：本地优先不可妥协"));
        assert_eq!(draft.item_type, "viewpoint");
    }

    #[test]
    fn change_type_label_falls_back_for_unknown_values() {
        assert_eq!(change_type_label("revised"), "修正");
        assert_eq!(change_type_label("模型自创词"), "变化");
    }

    #[test]
    fn empty_rebuild_marks_memory_scope_completed_and_clears_chunks() {
        // 同时验证 0018 迁移（memory_chunks / memory_chunks_search / scope 行）能正常执行。
        let root = std::env::temp_dir()
            .join("echo_memory_index_tests")
            .join(Uuid::new_v4().to_string());
        let library = ManagedLibrary::open(root).unwrap();
        rebuild_memory_index(&library).unwrap();
        let settings = library.repository().knowledge_settings().unwrap();
        let status = library
            .repository()
            .get_knowledge_index_status("memory", &settings.embedding_model)
            .unwrap();
        assert_eq!(status.status, "completed");
        assert_eq!(status.chunk_count, 0);
        assert!(library.repository().list_memory_chunks(&settings.embedding_model).unwrap().is_empty());
    }
}
