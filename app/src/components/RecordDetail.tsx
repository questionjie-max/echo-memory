import { convertFileSrc } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, useState } from "react";
import type { RecordNavigation } from "../App";
import type {
  AnalysisContent,
  AnalysisResultItem,
  AnalysisTemplate,
  LocalAiStatus,
  Project,
  RecordBrief,
  RecordStatus,
  TranscriptBlock,
  TranscriptSegment,
} from "../shared/types";
import {
  analyzeRecord,
  exportRecord,
  getLocalAiStatus,
  getRecord,
  latestAnalysis,
  listProjects,
  listAnalysisTemplates,
  listTranscriptBlocks,
  recordAudioPath,
  retranscribeRecord,
  transcribeRecord,
  updateRecordKnowledgeBase,
  updateRecordTitle,
  updateTranscriptSegment,
} from "../lib/tauri";

interface Props {
  record: RecordBrief;
  navigation: RecordNavigation | null;
  onChanged: (record?: RecordBrief) => void;
}

type DetailTab = "transcript" | "analysis";
type AudioLoadState = "loading" | "ready" | "error";

const TRANSCRIPTION_LANGUAGES = [
  ["zh", "中文"],
  ["auto", "自动检测"],
  ["en", "英语"],
  ["ja", "日语"],
  ["ko", "韩语"],
  ["fr", "法语"],
  ["de", "德语"],
  ["es", "西班牙语"],
];

export default function RecordDetail({ record, navigation, onChanged }: Props) {
  const audio = useRef<HTMLAudioElement>(null);
  const detailContent = useRef<HTMLDivElement>(null);
  const detailMenu = useRef<HTMLDetailsElement>(null);
  const pendingSeekMs = useRef<number | null>(null);
  const pendingPlay = useRef(false);
  const completionNotified = useRef(false);
  const [currentRecord, setCurrentRecord] = useState(record);
  const [source, setSource] = useState("");
  const [audioLoadState, setAudioLoadState] = useState<AudioLoadState>("loading");
  const [audioError, setAudioError] = useState("");
  const [segments, setSegments] = useState<TranscriptSegment[]>([]);
  const [blocks, setBlocks] = useState<TranscriptBlock[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  const [currentMs, setCurrentMs] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [analysis, setAnalysis] = useState<AnalysisContent | null>(null);
  const [activeTab, setActiveTab] = useState<DetailTab>("transcript");
  const [highlightedSegmentId, setHighlightedSegmentId] = useState<string | null>(null);
  const [visibleBlockCount, setVisibleBlockCount] = useState(80);
  const [expandedBlockIds, setExpandedBlockIds] = useState<Set<string>>(new Set());
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState(record.title);
  const [templates, setTemplates] = useState<AnalysisTemplate[]>([]);
  const [selectedTemplateId, setSelectedTemplateId] = useState(record.analysisTemplateId ?? "builtin-standard");
  const [aiStatus, setAiStatus] = useState<LocalAiStatus | null>(null);
  const [retranscribeOpen, setRetranscribeOpen] = useState(false);
  const [retranscribeLanguage, setRetranscribeLanguage] = useState("zh");
  const [retranscribeModelPath, setRetranscribeModelPath] = useState("");

  async function load() {
    try {
      const [latest, path, items, stored, knowledgeBases, templateItems, localAi] = await Promise.all([
        getRecord(record.id),
        recordAudioPath(record.id),
        listTranscriptBlocks(record.id),
        latestAnalysis(record.id),
        listProjects(),
        listAnalysisTemplates(),
        getLocalAiStatus(),
      ]);
      setCurrentRecord(latest);
      setAudioLoadState("loading");
      setAudioError("");
      setSource(convertFileSrc(path));
      setBlocks(items);
      setSegments(items.flatMap((item) => item.segments));
      setProjects(knowledgeBases);
      setAnalysis(stored ? parseAnalysis(stored.contentJson) : null);
      setTemplates(templateItems);
      setSelectedTemplateId(latest.analysisTemplateId ?? stored?.templateId ?? "builtin-standard");
      setAiStatus(localAi);
      setRetranscribeLanguage(localAi.settings.transcriptionLanguage);
      setRetranscribeModelPath(localAi.settings.whisperModelPath || localAi.whisperModelPath || "");
      if (!isProcessing(latest.status) && !completionNotified.current) {
        completionNotified.current = true;
        onChanged(latest);
      }
    } catch (reason) {
      setError(String(reason));
    }
  }

  useEffect(() => {
    setCurrentRecord(record);
    setSource("");
    setAudioLoadState("loading");
    setAudioError("");
    pendingSeekMs.current = null;
    pendingPlay.current = false;
    setVisibleBlockCount(80);
    setExpandedBlockIds(new Set());
    setHighlightedSegmentId(null);
    setActiveTab("transcript");
    setEditingTitle(false);
    setTitleDraft(record.title);
    setSelectedTemplateId(record.analysisTemplateId ?? "builtin-standard");
    setRetranscribeOpen(false);
    completionNotified.current = false;
    void load();
  }, [record.id]);

  useEffect(() => {
    if (!isProcessing(currentRecord.status)) return;
    const timer = window.setInterval(() => void load(), 2_000);
    return () => window.clearInterval(timer);
  }, [record.id, currentRecord.status]);

  useEffect(() => {
    if (!navigation || segments.length === 0) return;
    setActiveTab("transcript");
    setHighlightedSegmentId(navigation.targetSegmentId);
    if (navigation.startMs !== null && audio.current) {
      if (audio.current.readyState >= HTMLMediaElement.HAVE_METADATA) {
        audio.current.currentTime = navigation.startMs / 1000;
      } else {
        pendingSeekMs.current = navigation.startMs;
      }
      setCurrentMs(navigation.startMs);
    }
    window.setTimeout(() => {
      if (navigation.targetSegmentId) {
        const block = blocks.find((item) => item.segmentIds.includes(navigation.targetSegmentId!));
        document.getElementById(`block-${block?.id ?? navigation.targetSegmentId}`)?.scrollIntoView({ block: "center" });
      }
    }, 80);
  }, [navigation?.token, segments.length, blocks.length, source]);

  useEffect(() => {
    detailContent.current?.scrollTo({ top: 0 });
  }, [activeTab, record.id]);

  async function startTranscription() {
    setBusy(true);
    setError("");
    completionNotified.current = false;
    try {
      await transcribeRecord(record.id);
      setCurrentRecord((value) => ({ ...value, status: "preparing" }));
      onChanged();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function startAnalysis() {
    setBusy(true);
    setError("");
    completionNotified.current = false;
    try {
      await analyzeRecord(record.id, selectedTemplateId);
      setCurrentRecord((value) => ({ ...value, status: "analyzing", lastAnalysisError: null }));
      setActiveTab("analysis");
      onChanged();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function assignKnowledgeBase(knowledgeBaseId: string | null) {
    setBusy(true);
    setError("");
    try {
      const updated = await updateRecordKnowledgeBase(record.id, knowledgeBaseId);
      setCurrentRecord(updated);
      onChanged(updated);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function saveTitle() {
    const title = titleDraft.trim();
    if (!title || title === currentRecord.title) {
      setTitleDraft(currentRecord.title);
      setEditingTitle(false);
      return;
    }
    setBusy(true);
    setError("");
    try {
      const updated = await updateRecordTitle(record.id, title);
      setCurrentRecord(updated);
      setTitleDraft(updated.title);
      setEditingTitle(false);
      onChanged(updated);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function startRetranscription() {
    setBusy(true);
    setError("");
    completionNotified.current = false;
    try {
      await retranscribeRecord(record.id, retranscribeLanguage, retranscribeModelPath || null, "enhanced");
      setCurrentRecord((value) => ({ ...value, status: "preparing", hasAnalysis: false, analysisStatus: "stale" }));
      setRetranscribeOpen(false);
      setActiveTab("transcript");
      onChanged();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function chooseRetranscribeModel() {
    const selected = await openDialog({ multiple: false, directory: false, filters: [{ name: "Whisper 模型", extensions: ["bin"] }] });
    if (typeof selected === "string") setRetranscribeModelPath(selected);
  }

  async function exportOne(format: "md" | "txt") {
    if (detailMenu.current) detailMenu.current.open = false;
    const destination = await saveDialog({
      title: `导出${currentRecord.title}`,
      defaultPath: `${currentRecord.title}.${format}`,
      filters: [{ name: format === "md" ? "Markdown" : "文本", extensions: [format] }],
    });
    if (!destination) return;
    setBusy(true);
    setError("");
    try {
      await exportRecord(record.id, destination, format);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  function seek(ms: number, play = true) {
    if (!audio.current) return;
    setCurrentMs(ms);
    setAudioError("");
    if (audio.current.readyState < HTMLMediaElement.HAVE_METADATA) {
      pendingSeekMs.current = ms;
      pendingPlay.current = play;
      setAudioLoadState("loading");
      return;
    }
    audio.current.currentTime = ms / 1000;
    if (play) {
      void audio.current.play().catch((reason) => {
        setAudioLoadState("error");
        setAudioError(`无法播放音频：${String(reason)}`);
      });
    }
  }

  function audioReady(element: HTMLAudioElement) {
    setAudioLoadState("ready");
    setAudioError("");
    if (pendingSeekMs.current !== null) {
      element.currentTime = pendingSeekMs.current / 1000;
      setCurrentMs(pendingSeekMs.current);
      pendingSeekMs.current = null;
    }
    if (pendingPlay.current) {
      pendingPlay.current = false;
      void element.play().catch((reason) => {
        setAudioLoadState("error");
        setAudioError(`无法播放音频：${String(reason)}`);
      });
    }
  }

  function audioFailed(element: HTMLAudioElement) {
    const code = element.error?.code;
    const message = code === MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED
      ? "当前音频格式无法由系统播放器读取"
      : code === MediaError.MEDIA_ERR_DECODE
        ? "音频解码失败，原文件仍已保留"
        : "音频载入失败，请重新打开录音";
    setAudioLoadState("error");
    setAudioError(message);
  }

  async function save(segment: TranscriptSegment, text: string) {
    try {
      await updateTranscriptSegment(segment.id, text);
      const refreshed = await listTranscriptBlocks(record.id);
      setBlocks(refreshed);
      setSegments(refreshed.flatMap((item) => item.segments));
      setCurrentRecord((value) => ({ ...value, hasAnalysis: false, analysisStatus: "stale" }));
      onChanged();
    } catch (reason) {
      setError(String(reason));
    }
  }

  const activeProjects = projects.filter((project) => project.status === "active" || project.id === currentRecord.projectId);

  return (
    <section className="detail-pane" aria-label="录音详情">
      <header className="detail-header material">
        <div className="detail-title">
          <span className={`status-dot status-${statusTone(currentRecord.status)}`} aria-hidden="true" />
          <div>
            <p className="pane-eyebrow">录音详情 · {statusText(currentRecord)}</p>
            {editingTitle ? (
              <form className="title-editor" onSubmit={(event) => { event.preventDefault(); void saveTitle(); }}>
                <input
                  autoFocus
                  value={titleDraft}
                  maxLength={160}
                  aria-label="录音标题"
                  onChange={(event) => setTitleDraft(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Escape") {
                      setTitleDraft(currentRecord.title);
                      setEditingTitle(false);
                    }
                  }}
                />
                <button type="submit" disabled={busy || !titleDraft.trim()}>保存</button>
                <button type="button" disabled={busy} onClick={() => { setTitleDraft(currentRecord.title); setEditingTitle(false); }}>取消</button>
              </form>
            ) : (
              <div className="title-display">
                <h2 title={currentRecord.title}>{currentRecord.title}</h2>
                <button type="button" disabled={busy} onClick={() => { setTitleDraft(currentRecord.title); setEditingTitle(true); }}>重命名</button>
              </div>
            )}
          </div>
        </div>
        <div className="detail-actions">
          <label className="knowledge-select">
            <span>知识库</span>
            <select
              value={currentRecord.projectId ?? ""}
              disabled={busy || !currentRecord.hasTranscript}
              onChange={(event) => void assignKnowledgeBase(event.target.value || null)}
            >
              <option value="">未归档</option>
              {activeProjects.map((project) => <option value={project.id} key={project.id}>{project.name}</option>)}
            </select>
          </label>
          <details className="detail-menu" ref={detailMenu}>
            <summary>操作</summary>
            <div className="detail-menu-popover material">
              <button type="button" disabled={busy || isProcessing(currentRecord.status)} onClick={() => { if (detailMenu.current) detailMenu.current.open = false; setRetranscribeOpen(true); }}>增强重新转写…</button>
              <button type="button" disabled={busy || !currentRecord.hasTranscript} onClick={() => void exportOne("md")}>导出 Markdown</button>
              <button type="button" disabled={busy || !currentRecord.hasTranscript} onClick={() => void exportOne("txt")}>导出 TXT</button>
            </div>
          </details>
        </div>
      </header>

      {source && (
        <div className="audio-bar">
          <audio
            ref={audio}
            controls
            preload="metadata"
            src={source}
            onLoadStart={() => setAudioLoadState("loading")}
            onLoadedMetadata={(event) => audioReady(event.currentTarget)}
            onCanPlay={(event) => audioReady(event.currentTarget)}
            onError={(event) => audioFailed(event.currentTarget)}
            onTimeUpdate={(event) => setCurrentMs(event.currentTarget.currentTime * 1000)}
          />
          <div className={`audio-status audio-${audioLoadState}`} role="status">
            <span>{audioLoadState === "ready" ? "音频已就绪" : audioLoadState === "loading" ? "正在载入音频" : audioError}</span>
            <span>总时长 {formatTime(currentRecord.audioDurationMs)}</span>
          </div>
        </div>
      )}

      {isProcessing(currentRecord.status) && (
        <div className="processing-progress" role="status">
          <div><strong>{processingStageText(currentRecord.processingStage)}</strong><span>{progressText(currentRecord)}</span></div>
          <div className="progress-track"><span style={{ width: `${processingPercent(currentRecord)}%` }} /></div>
        </div>
      )}

      <div className="detail-tabs" role="tablist" aria-label="录音内容">
        <button type="button" role="tab" aria-selected={activeTab === "transcript"} className={activeTab === "transcript" ? "selected" : ""} onClick={() => setActiveTab("transcript")}>逐字稿</button>
        <button type="button" role="tab" aria-selected={activeTab === "analysis"} className={activeTab === "analysis" ? "selected" : ""} onClick={() => setActiveTab("analysis")}>文稿分析</button>
      </div>

      {error && <p className="detail-error">{error}</p>}

      <div className="detail-content" ref={detailContent}>
        {activeTab === "transcript" ? (
          segments.length === 0 ? (
            <div className="detail-placeholder">
              <p>{isProcessing(currentRecord.status) ? "正在本机转写，可继续使用其他功能。" : currentRecord.status === "failed" ? "本地转写失败，音频仍已安全保存。" : "尚未生成逐字稿。"}</p>
              {!isProcessing(currentRecord.status) && <button type="button" className="primary-button" disabled={busy} onClick={() => void startTranscription()}>重新转写</button>}
            </div>
          ) : (
            <>
              <div className="transcript-block-list">
                {blocks.slice(0, visibleBlockCount).map((block) => (
                  <TranscriptBlockRow
                    key={block.id}
                    block={block}
                    active={currentMs >= block.startMs && currentMs < block.endMs}
                    highlightedSegmentId={highlightedSegmentId && block.segmentIds.includes(highlightedSegmentId) ? highlightedSegmentId : null}
                    expanded={expandedBlockIds.has(block.id)}
                    onToggle={() => setExpandedBlockIds((current) => {
                      const next = new Set(current);
                      if (next.has(block.id)) next.delete(block.id); else next.add(block.id);
                      return next;
                    })}
                    onSeek={seek}
                    onSave={save}
                  />
                ))}
              </div>
              {visibleBlockCount < blocks.length && (
                <button type="button" className="secondary-button load-more" onClick={() => setVisibleBlockCount((count) => Math.min(count + 80, blocks.length))}>
                  加载更多（{visibleBlockCount} / {blocks.length}）
                </button>
              )}
            </>
          )
        ) : (
          <div className="analysis-view">
            <div className="analysis-action">
              <div>
                <strong>{analysis ? "更新文稿分析" : "本地文稿分析"}</strong>
                <p>使用本机 Qwen 生成摘要、观点、决策和待办。</p>
              </div>
              <div className="analysis-controls">
                <label className="template-select">
                  <span>分析模板</span>
                  <select value={selectedTemplateId} disabled={busy || isProcessing(currentRecord.status)} onChange={(event) => setSelectedTemplateId(event.target.value)}>
                    {templates.map((template) => <option value={template.id} key={template.id}>{template.name}</option>)}
                  </select>
                </label>
                <button type="button" className="primary-button" disabled={busy || segments.length === 0 || isProcessing(currentRecord.status)} onClick={() => void startAnalysis()}>
                  {currentRecord.status === "analyzing" ? "分析中…" : analysis ? "重新分析" : "开始分析"}
                </button>
              </div>
            </div>
            {currentRecord.analysisStatus === "stale" && (
              <p className="analysis-warning">逐字稿已变化，当前分析基于旧版本，请重新分析。</p>
            )}
            {currentRecord.lastAnalysisError && !isProcessing(currentRecord.status) && (
              <p className="analysis-error">{currentRecord.lastAnalysisError}</p>
            )}
            {analysis ? <AnalysisView analysis={analysis} onSeek={(ms, segmentId) => {
              setHighlightedSegmentId(segmentId);
              setActiveTab("transcript");
              window.setTimeout(() => {
                const block = blocks.find((item) => item.segmentIds.includes(segmentId));
                document.getElementById(`block-${block?.id ?? segmentId}`)?.scrollIntoView({ block: "center" });
                seek(ms);
              }, 40);
            }} /> : (
              <div className="detail-placeholder"><p>{segments.length ? "还没有文稿分析。" : "逐字稿完成后即可开始分析。"}</p></div>
            )}
          </div>
        )}
      </div>
      {retranscribeOpen && (
        <div className="dialog-scrim" role="presentation" onMouseDown={(event) => event.target === event.currentTarget && setRetranscribeOpen(false)}>
          <section className="retranscribe-dialog material" role="dialog" aria-modal="true" aria-label="增强重新转写">
            <header>
              <div><p className="pane-eyebrow">生成新版本</p><h3>增强重新转写</h3></div>
              <button type="button" className="close-button" aria-label="关闭" title="关闭" onClick={() => setRetranscribeOpen(false)}>×</button>
            </header>
            <p>使用音频预处理、智能分块和简体规范化。旧逐字稿会保留，新版本完成后旧分析将标记为需要更新。</p>
            <label>
              <span>转写语言</span>
              <select value={retranscribeLanguage} onChange={(event) => setRetranscribeLanguage(event.target.value)}>
                {TRANSCRIPTION_LANGUAGES.map(([value, label]) => <option value={value} key={value}>{label}</option>)}
              </select>
            </label>
            <label>
              <span>Whisper 模型</span>
              <div className="path-control"><input readOnly value={retranscribeModelPath} placeholder="使用应用默认模型" /><button type="button" onClick={() => void chooseRetranscribeModel()}>选择</button></div>
            </label>
            {aiStatus?.whisperModelSource && <small>{aiStatus.whisperModelSource}</small>}
            <div className="dialog-actions">
              <button type="button" className="secondary-button" disabled={busy} onClick={() => setRetranscribeOpen(false)}>取消</button>
              <button type="button" className="primary-button" disabled={busy || !retranscribeLanguage} onClick={() => void startRetranscription()}>{busy ? "启动中…" : "开始增强转写"}</button>
            </div>
          </section>
        </div>
      )}
    </section>
  );
}

function AnalysisView({ analysis, onSeek }: { analysis: AnalysisContent; onSeek: (ms: number, segmentId: string) => void }) {
  const groups: Array<[string, AnalysisResultItem[] | undefined, string]> = [
    ["关键观点", analysis.key_points, "未识别出足够可靠的关键观点。"],
    ["决策", analysis.decisions, "未识别出明确决策。"],
    ["待办", analysis.action_items, "未识别出明确待办。"],
    ["未解决问题", analysis.open_questions, "未识别出明确的未解决问题。"],
  ];
  return (
    <div className="analysis-sections">
      {analysis.quality_warning && <p className="analysis-warning">分析不完整：{analysis.quality_warning}</p>}
      <section className="analysis-section">
        <h3>摘要</h3>
        <p>{analysis.summary || "暂未生成有效摘要。"}</p>
      </section>
      {groups.map(([name, items, emptyText]) => (
        <section className="analysis-section" key={name}>
          <h3>{name}</h3>
          {items?.length ? (
            <ul className="analysis-list">
              {items.map((item, index) => {
                const segmentId = item.citation_segment_ids?.[0];
                const locatable = typeof item.start_ms === "number" && Boolean(segmentId);
                return (
                  <li key={`${name}-${index}`}>
                    <button type="button" disabled={!locatable} onClick={() => locatable && onSeek(item.start_ms!, segmentId!)}>
                      <strong>{item.text}</strong>
                      <span>{item.quote_text && segmentId ? `“${item.quote_text}” · ${formatTime(item.start_ms ?? 0)}` : "无可靠出处"}</span>
                    </button>
                  </li>
                );
              })}
            </ul>
          ) : <p className="analysis-empty">{emptyText}</p>}
        </section>
      ))}
      {analysis.custom_sections?.map((section) => (
        <section className="analysis-section custom-analysis-section" key={section.key}>
          <h3>{section.title}</h3>
          {section.format === "paragraph" ? (
            <p>{section.text || "未识别出可靠内容。"}</p>
          ) : section.items?.length ? (
            <ul className="analysis-list">
              {section.items.map((item, index) => {
                const segmentId = item.citation_segment_ids?.[0];
                const locatable = typeof item.start_ms === "number" && Boolean(segmentId);
                return (
                  <li key={`${section.key}-${index}`}>
                    <button type="button" disabled={!locatable} onClick={() => locatable && onSeek(item.start_ms!, segmentId!)}>
                      <strong>{item.text}</strong>
                      <span>{item.quote_text && segmentId ? `“${item.quote_text}” · ${formatTime(item.start_ms ?? 0)}` : "无可靠出处"}</span>
                    </button>
                  </li>
                );
              })}
            </ul>
          ) : <p className="analysis-empty">未识别出可靠内容。</p>}
        </section>
      ))}
    </div>
  );
}

function SegmentRow({ segment, active, highlighted, onSeek, onSave }: {
  segment: TranscriptSegment;
  active: boolean;
  highlighted: boolean;
  onSeek: (ms: number) => void;
  onSave: (segment: TranscriptSegment, text: string) => void;
}) {
  const effective = segment.editedText ?? segment.normalizedText ?? segment.originalText;
  const [text, setText] = useState(effective);
  useEffect(() => setText(segment.editedText ?? segment.normalizedText ?? segment.originalText), [segment]);
  return (
    <article id={`segment-${segment.id}`} className={`segment-row${active ? " active" : ""}${highlighted ? " search-hit" : ""}`}>
      <button type="button" onClick={() => onSeek(segment.startMs)} className="time-button">{formatTime(segment.startMs)}</button>
      <div>
        <p className="speaker-label">{segment.speakerLabel ?? "说话人未知"}</p>
        <textarea
          value={text}
          onChange={(event) => setText(event.target.value)}
          onBlur={() => {
            if (text !== effective) void onSave(segment, text);
          }}
          aria-label={`片段 ${segment.sequence + 1} 文本`}
        />
        {segment.editedText !== null && <small>原文：{segment.originalText}</small>}
      </div>
    </article>
  );
}

function TranscriptBlockRow({ block, active, highlightedSegmentId, expanded, onToggle, onSeek, onSave }: {
  block: TranscriptBlock;
  active: boolean;
  highlightedSegmentId: string | null;
  expanded: boolean;
  onToggle: () => void;
  onSeek: (ms: number) => void;
  onSave: (segment: TranscriptSegment, text: string) => void;
}) {
  return <article id={`block-${block.id}`} className={`transcript-block${active ? " active" : ""}${highlightedSegmentId ? " search-hit" : ""}`}>
    <div className="transcript-block-header">
      <button type="button" className="time-button" onClick={() => onSeek(block.startMs)}>{formatTime(block.startMs)}</button>
      <span>{block.speakerLabel ?? "说话人未知"}</span>
      <span>{Math.max(1, Math.round((block.endMs - block.startMs) / 1000))} 秒</span>
      <button type="button" className="block-edit-button" onClick={onToggle}>{expanded ? "收起" : "编辑"}</button>
    </div>
    {!expanded ? <p>{block.text}</p> : <div className="block-segments">{block.segments.map((segment) => <SegmentRow key={segment.id} segment={segment} active={false} highlighted={segment.id === highlightedSegmentId} onSeek={onSeek} onSave={onSave} />)}</div>}
  </article>;
}

function parseAnalysis(content: string): AnalysisContent | null {
  try {
    return JSON.parse(content) as AnalysisContent;
  } catch {
    return null;
  }
}

function formatTime(ms: number) {
  const seconds = Math.floor(ms / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

function isProcessing(status: RecordStatus) {
  return status === "preparing" || status === "transcribing" || status === "analyzing";
}

function statusText(record: RecordBrief) {
  if (record.status === "analyzing") return "本地分析中";
  if (record.status === "transcribing" || record.status === "preparing") return "本地转写中";
  if (record.status === "failed") return "转写失败";
  if (record.lastAnalysisError && !record.hasAnalysis) return "分析失败";
  if (record.analysisStatus === "stale") return "分析需要更新";
  if (record.analysisStatus === "incomplete") return "分析不完整";
  if (!record.hasAnalysis) return "待分析";
  if (!record.projectId) return "待归档";
  return "已完成";
}

function statusTone(status: RecordStatus) {
  if (status === "failed") return "error";
  if (isProcessing(status)) return "progress";
  return "done";
}

function processingStageText(stage: string | null) {
  const labels: Record<string, string> = {
    preprocessing: "正在预处理音频",
    chunking: "正在寻找分块边界",
    transcribing: "正在本机转写",
    merging: "正在合并转写结果",
    normalizing: "正在转换为简体中文",
    analyzing: "正在分段分析",
    validating: "正在校验引用",
    saving: "正在保存结果",
  };
  return stage ? labels[stage] ?? "正在本机处理" : "正在本机处理";
}

function progressText(record: RecordBrief) {
  return record.progressTotal > 1 ? `${Math.min(record.progressCurrent + 1, record.progressTotal)} / ${record.progressTotal}` : "";
}

function processingPercent(record: RecordBrief) {
  if (record.progressTotal <= 0) return 12;
  return Math.max(4, Math.min(100, Math.round(record.progressCurrent / record.progressTotal * 100)));
}
