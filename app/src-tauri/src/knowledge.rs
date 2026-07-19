use crate::error::{AppError, AppResult};
use crate::library::ManagedLibrary;
use crate::transcript::effective_text;
use crate::types::{
    KnowledgeAnswer, KnowledgeAnswerCitation, KnowledgeChunkInput, KnowledgeChunkRecord,
    KnowledgeIndexStatus, TranscriptSegment,
};
use chrono::Utc;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::HashMap;
use uuid::Uuid;

const OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";

#[derive(Debug, Clone)]
struct ChunkDraft {
    segment_ids: Vec<String>,
    body: String,
    start_ms: i64,
    end_ms: i64,
    speaker_label: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnswerDraft {
    answer: String,
    #[serde(default)]
    citation_chunk_ids: Vec<String>,
    #[serde(default)]
    insufficient_evidence: bool,
}

pub fn scope_key(project_id: Option<&str>, unfiled_only: bool) -> String {
    match (project_id, unfiled_only) {
        (Some(id), _) => format!("project:{id}"),
        (None, true) => "unfiled".to_owned(),
        (None, false) => "all".to_owned(),
    }
}

pub fn rebuild_scope(
    library: &ManagedLibrary,
    project_id: Option<&str>,
    unfiled_only: bool,
) -> AppResult<KnowledgeIndexStatus> {
    let repository = library.repository();
    let settings = repository.knowledge_settings()?;
    let key = scope_key(project_id, unfiled_only);
    let records = repository
        .list_records(project_id, unfiled_only)?
        .into_iter()
        .filter(|record| record.has_transcript)
        .collect::<Vec<_>>();
    let mut status = KnowledgeIndexStatus {
        scope_key: key,
        status: "indexing".to_owned(),
        total_records: records.len() as i64,
        processed_records: 0,
        chunk_count: 0,
        embedding_model: settings.embedding_model.clone(),
        last_error: None,
        updated_at: Utc::now().to_rfc3339(),
    };
    repository.save_knowledge_index_status(&status)?;
    if let Err(error) = ensure_model_installed(&settings.embedding_model) {
        status.status = "failed".to_owned();
        status.last_error = Some(error.to_string());
        status.updated_at = Utc::now().to_rfc3339();
        repository.save_knowledge_index_status(&status)?;
        return Err(error);
    }

    for record in records {
        let result = rebuild_record(library, &record.id, &settings.embedding_model);
        match result {
            Ok(count) => {
                status.processed_records += 1;
                status.chunk_count += count as i64;
                status.updated_at = Utc::now().to_rfc3339();
                repository.save_knowledge_index_status(&status)?;
            }
            Err(error) => {
                status.status = "failed".to_owned();
                status.last_error = Some(error.to_string());
                status.updated_at = Utc::now().to_rfc3339();
                repository.save_knowledge_index_status(&status)?;
                return Err(error);
            }
        }
    }
    status.status = "completed".to_owned();
    status.updated_at = Utc::now().to_rfc3339();
    repository.save_knowledge_index_status(&status)?;
    Ok(status)
}

pub fn rebuild_record(
    library: &ManagedLibrary,
    record_id: &str,
    embedding_model: &str,
) -> AppResult<usize> {
    let repository = library.repository();
    let record = repository.get_record(record_id)?;
    let version = repository.latest_transcript_version(record_id)?;
    let segments = repository.list_transcript_segments(record_id)?;
    let drafts = chunk_segments(&segments);
    let texts = drafts
        .iter()
        .map(|chunk| chunk.body.clone())
        .collect::<Vec<_>>();
    let embeddings = embed_texts(embedding_model, &texts)?;
    if embeddings.len() != drafts.len() {
        return Err(AppError::Analysis(
            "嵌入模型返回的向量数量不正确".to_owned(),
        ));
    }
    let inputs = drafts
        .into_iter()
        .zip(embeddings)
        .map(|(chunk, embedding)| {
            let content_hash = format!("{:x}", Sha256::digest(chunk.body.as_bytes()));
            KnowledgeChunkInput {
                id: Uuid::new_v4().to_string(),
                record_id: record_id.to_owned(),
                project_id: record.project_id.clone(),
                transcript_version_id: version.id.clone(),
                segment_ids: chunk.segment_ids,
                body: chunk.body,
                start_ms: chunk.start_ms,
                end_ms: chunk.end_ms,
                speaker_label: chunk.speaker_label,
                content_hash,
                embedding_model: embedding_model.to_owned(),
                embedding,
            }
        })
        .collect::<Vec<_>>();
    repository.replace_knowledge_chunks(record_id, &inputs)?;
    Ok(inputs.len())
}

pub fn incremental_rebuild_record(library: &ManagedLibrary, record_id: &str) -> AppResult<()> {
    let repository = library.repository();
    let settings = repository.knowledge_settings()?;
    if !repository.has_knowledge_index(&settings.embedding_model)? {
        return Ok(());
    }
    ensure_model_installed(&settings.embedding_model)?;
    rebuild_record(library, record_id, &settings.embedding_model)?;
    repository.refresh_knowledge_index_counts(&settings.embedding_model)
}

pub fn ask(
    library: &ManagedLibrary,
    project_id: Option<&str>,
    unfiled_only: bool,
    question: &str,
) -> AppResult<KnowledgeAnswer> {
    let question = question.trim();
    if question.chars().count() < 2 || question.chars().count() > 500 {
        return Err(AppError::Invalid("问题应为 2 到 500 个字符".to_owned()));
    }
    let repository = library.repository();
    let settings = repository.knowledge_settings()?;
    let status = repository.get_knowledge_index_status(
        &scope_key(project_id, unfiled_only),
        &settings.embedding_model,
    )?;
    if status.status != "completed" {
        return Err(AppError::Analysis(
            "知识索引尚未完成，请先建立或更新索引".to_owned(),
        ));
    }
    let chunks =
        repository.list_knowledge_chunks(project_id, unfiled_only, &settings.embedding_model)?;
    if chunks.is_empty() {
        return Ok(insufficient_answer());
    }
    let query_embedding = embed_texts(&settings.embedding_model, &[question.to_owned()])?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::Analysis("嵌入模型未返回问题向量".to_owned()))?;
    let ranked = hybrid_rank(
        repository,
        question,
        &query_embedding,
        &chunks,
        project_id,
        unfiled_only,
    )?;
    if ranked.is_empty() || ranked[0].1 < 0.25 {
        return Ok(insufficient_answer());
    }
    let selected = ranked
        .into_iter()
        .take(10)
        .map(|(index, _)| &chunks[index])
        .collect::<Vec<_>>();
    let context = selected
        .iter()
        .map(|chunk| {
            format!(
                "[chunk:{}] 录音：{}，时间：{}-{}\n{}",
                chunk.id, chunk.record_title, chunk.start_ms, chunk.end_ms, chunk.body
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let prompt = format!(
        "你是本地知识库问答助手。只能依据给定知识片段回答。仅返回 JSON：{{\"answer\":\"回答\",\"citation_chunk_ids\":[\"chunk id\"],\"insufficient_evidence\":false}}。\n\
         每个事实必须由 citation_chunk_ids 支持；引用只能使用下方出现的 chunk id。证据不足时 answer 必须为“没有找到可靠依据”，citation_chunk_ids 为空且 insufficient_evidence=true。禁止使用外部知识或猜测。\n\
         问题：{question}\n\n知识片段：\n{context}"
    );
    let output = generate_answer(&settings.analysis_model, &prompt)?;
    validate_answer(output, &selected)
}

fn chunk_segments(segments: &[TranscriptSegment]) -> Vec<ChunkDraft> {
    let mut chunks = Vec::new();
    let mut current: Vec<&TranscriptSegment> = Vec::new();
    let mut length = 0usize;
    for segment in segments {
        let text = effective_text(segment);
        let text_length = text.chars().count();
        let duration = current
            .first()
            .map_or(0, |first| segment.end_ms - first.start_ms);
        if !current.is_empty()
            && (length + text_length > 800 || (length >= 300 && duration > 90_000))
        {
            chunks.push(build_chunk(&current));
            current.clear();
            length = 0;
        }
        current.push(segment);
        length += text_length;
    }
    if !current.is_empty() {
        chunks.push(build_chunk(&current));
    }
    chunks
}

fn build_chunk(segments: &[&TranscriptSegment]) -> ChunkDraft {
    let body = segments
        .iter()
        .map(|segment| {
            let speaker = segment.speaker_label.as_deref().unwrap_or("未知");
            let text = effective_text(segment);
            format!("{speaker}：{text}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let speaker_label = segments
        .iter()
        .filter_map(|segment| segment.speaker_label.as_deref())
        .find(|speaker| *speaker != "未知")
        .map(str::to_owned);
    ChunkDraft {
        segment_ids: segments.iter().map(|segment| segment.id.clone()).collect(),
        body,
        start_ms: segments.first().map_or(0, |segment| segment.start_ms),
        end_ms: segments.last().map_or(0, |segment| segment.end_ms),
        speaker_label,
    }
}

fn embed_texts(model: &str, inputs: &[String]) -> AppResult<Vec<Vec<f32>>> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let response: serde_json::Value = ureq::post(&format!("{OLLAMA_BASE_URL}/api/embed"))
        .send_json(serde_json::json!({ "model": model, "input": inputs }))
        .map_err(|error| AppError::Analysis(format!("本地嵌入请求失败：{error}")))?
        .into_json()
        .map_err(|error| AppError::Analysis(format!("无法读取嵌入响应：{error}")))?;
    serde_json::from_value(
        response
            .get("embeddings")
            .cloned()
            .ok_or_else(|| AppError::Analysis("嵌入模型未返回 embeddings".to_owned()))?,
    )
    .map_err(|error| AppError::Analysis(format!("嵌入向量格式无效：{error}")))
}

fn ensure_model_installed(model: &str) -> AppResult<()> {
    let tags: serde_json::Value = ureq::get(&format!("{OLLAMA_BASE_URL}/api/tags"))
        .call()
        .map_err(|_| AppError::Analysis("Ollama 未启动，无法建立知识索引".to_owned()))?
        .into_json()
        .map_err(|_| AppError::Analysis("无法读取 Ollama 模型列表".to_owned()))?;
    let installed = tags
        .get("models")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| item.get("name").and_then(serde_json::Value::as_str) == Some(model))
        });
    if installed {
        Ok(())
    } else {
        Err(AppError::Analysis(format!("尚未安装嵌入模型“{model}”")))
    }
}

fn hybrid_rank(
    repository: &crate::db::repository::LibraryRepository,
    question: &str,
    query: &[f32],
    chunks: &[KnowledgeChunkRecord],
    project_id: Option<&str>,
    unfiled_only: bool,
) -> AppResult<Vec<(usize, f32)>> {
    let mut vector_order = chunks
        .iter()
        .enumerate()
        .map(|(index, chunk)| (index, cosine_similarity(query, &chunk.embedding)))
        .collect::<Vec<_>>();
    vector_order.sort_by(|left, right| right.1.partial_cmp(&left.1).unwrap_or(Ordering::Equal));
    let mut scores = HashMap::<usize, f32>::new();
    for (rank, (index, similarity)) in vector_order.iter().take(20).enumerate() {
        scores.insert(*index, 1.0 / (60.0 + rank as f32) + similarity.max(0.0));
    }
    if let Ok(results) = repository.search(question, project_id, unfiled_only, 20) {
        for (rank, result) in results.iter().enumerate() {
            for (index, chunk) in chunks.iter().enumerate() {
                let overlaps = result.start_ms.is_none_or(|start| {
                    start <= chunk.end_ms && result.end_ms.unwrap_or(start) >= chunk.start_ms
                });
                if chunk.record_id == result.record_id && overlaps {
                    *scores.entry(index).or_default() += 1.0 / (60.0 + rank as f32);
                }
            }
        }
    }
    let mut ranked = scores.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.1.partial_cmp(&left.1).unwrap_or(Ordering::Equal));
    Ok(ranked)
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return -1.0;
    }
    let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        -1.0
    } else {
        dot / (left_norm * right_norm)
    }
}

fn generate_answer(model: &str, prompt: &str) -> AppResult<AnswerDraft> {
    let format = serde_json::json!({
        "type": "object",
        "properties": {
            "answer": { "type": "string" },
            "citation_chunk_ids": { "type": "array", "items": { "type": "string" }, "maxItems": 10 },
            "insufficient_evidence": { "type": "boolean" }
        },
        "required": ["answer", "citation_chunk_ids", "insufficient_evidence"]
    });
    let response: serde_json::Value = ureq::post(&format!("{OLLAMA_BASE_URL}/api/generate"))
        .send_json(serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": false,
            "format": format,
            "options": { "num_ctx": 16384, "num_predict": 1024, "temperature": 0.1 }
        }))
        .map_err(|error| AppError::Analysis(format!("本地知识问答失败：{error}")))?
        .into_json()
        .map_err(|error| AppError::Analysis(format!("无法读取知识问答响应：{error}")))?;
    let output = response
        .get("response")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AppError::Analysis("知识问答未返回 JSON".to_owned()))?;
    serde_json::from_str(output)
        .map_err(|error| AppError::Analysis(format!("知识问答 JSON 无效：{error}")))
}

fn validate_answer(
    draft: AnswerDraft,
    selected: &[&KnowledgeChunkRecord],
) -> AppResult<KnowledgeAnswer> {
    if draft.insufficient_evidence {
        return Ok(insufficient_answer());
    }
    let available = selected
        .iter()
        .map(|chunk| (chunk.id.as_str(), *chunk))
        .collect::<HashMap<_, _>>();
    let mut citations = Vec::new();
    for chunk_id in draft.citation_chunk_ids {
        let chunk = available
            .get(chunk_id.as_str())
            .ok_or_else(|| AppError::Analysis("模型返回了不属于当前知识库的引用".to_owned()))?;
        let segment_id = chunk
            .segment_ids
            .first()
            .cloned()
            .ok_or_else(|| AppError::Analysis("知识片段缺少原文引用".to_owned()))?;
        citations.push(KnowledgeAnswerCitation {
            chunk_id,
            record_id: chunk.record_id.clone(),
            record_title: chunk.record_title.clone(),
            quote_text: chunk.body.clone(),
            segment_id,
            start_ms: chunk.start_ms,
            end_ms: chunk.end_ms,
        });
    }
    if draft.answer.trim().is_empty() || citations.is_empty() {
        return Ok(insufficient_answer());
    }
    Ok(KnowledgeAnswer {
        answer: draft.answer.trim().to_owned(),
        citations,
        insufficient_evidence: false,
    })
}

fn insufficient_answer() -> KnowledgeAnswer {
    KnowledgeAnswer {
        answer: "没有找到可靠依据".to_owned(),
        citations: Vec::new(),
        insufficient_evidence: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking_preserves_segment_bounds() {
        let segments = (0..6)
            .map(|index| TranscriptSegment {
                id: format!("s{index}"),
                record_id: "r".to_owned(),
                sequence: index,
                speaker_label: Some("未知".to_owned()),
                start_ms: index * 1_000,
                end_ms: index * 1_000 + 900,
                original_text: "这是用于构建知识片段的一段有效逐字稿内容。".repeat(8),
                normalized_text: None,
                normalization_version: None,
                edited_text: None,
            })
            .collect::<Vec<_>>();
        let chunks = chunk_segments(&segments);
        assert!(chunks.len() >= 2);
        assert_eq!(chunks[0].segment_ids[0], "s0");
        assert_eq!(chunks.last().unwrap().end_ms, 5_900);
    }

    #[test]
    fn cosine_similarity_rejects_dimension_mismatch() {
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), -1.0);
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 0.001);
    }

    #[test]
    fn answer_rejects_unknown_citation() {
        let chunk = KnowledgeChunkRecord {
            id: "known".to_owned(),
            record_id: "r".to_owned(),
            record_title: "标题".to_owned(),
            project_id: None,
            segment_ids: vec!["s".to_owned()],
            body: "证据".to_owned(),
            start_ms: 0,
            end_ms: 1,
            embedding_model: "m".to_owned(),
            embedding: vec![1.0],
        };
        let result = validate_answer(
            AnswerDraft {
                answer: "结论".to_owned(),
                citation_chunk_ids: vec!["other".to_owned()],
                insufficient_evidence: false,
            },
            &[&chunk],
        );
        assert!(result.is_err());
    }
}
