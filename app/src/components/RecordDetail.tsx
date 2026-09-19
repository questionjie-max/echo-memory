import { convertFileSrc } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { PauseIcon, PlayIcon, SkipBackIcon, SkipForwardIcon } from "./icons";
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
  RelatedRecord,
  SpeakerSummary,
  TranscriptionEngineStatus,
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
  relatedRecords,
  getRecordSpeakers,
  renameRecordSpeaker,
  getTranscriptionEngineStatus,
  exportRecordToOutput,
  correctTranscript,
  retranscribeRecord,
  transcribeRecord,
  updateRecordKnowledgeBase,
  updateRecordTitle,
  updateTranscriptSegment,
} from "../lib/tauri";
import { listen } from "@tauri-apps/api/event";
import { formatMinutesSeconds as formatTime, isProcessingStatus as isProcessing } from "../lib/format";
import { Field, List, ListRow, Segmented } from "./SettingsKit";

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
  const loadRequestId = useRef(0);
  const [currentRecord, setCurrentRecord] = useState(record);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const records = await relatedRecords(currentRecord.id, 5);
        if (!cancelled) setRelated(records);
      } catch {
        if (!cancelled) setRelated([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [currentRecord.id, currentRecord.analysisStatus]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [list, engine] = await Promise.all([
          getRecordSpeakers(currentRecord.id),
          getTranscriptionEngineStatus(),
        ]);
        if (!cancelled) {
          setSpeakers(list);
          setEngineStatus(engine);
        }
      } catch {
        if (!cancelled) {
          setSpeakers([]);
          setEngineStatus(null);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [currentRecord.id, currentRecord.hasTranscript]);

  useEffect(() => {
    const stop = listen<{ recordId: string; ok: boolean; message: string }>(
      "transcript-corrected",
      (event) => {
        if (event.payload.recordId === currentRecord.id) {
          setCorrecting(false);
          void load();
        }
      },
    );
    return () => {
      void stop.then((unlisten) => unlisten());
    };
  }, [currentRecord.id]);

  async function startCorrection() {
    setCorrecting(true);
    try {
      await correctTranscript(currentRecord.id);
    } catch (reason) {
      setCorrecting(false);
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  }

  const [source, setSource] = useState("");
  const [audioLoadState, setAudioLoadState] = useState<AudioLoadState>("loading");
  const [audioError, setAudioError] = useState("");
  const [isPlaying, setIsPlaying] = useState(false);
  const [playbackRate, setPlaybackRate] = useState(1);
  const [volume, setVolume] = useState(1);
  const [durationMs, setDurationMs] = useState<number | null>(null);
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
  const [retranscribeEngine, setRetranscribeEngine] = useState<"embedded" | "whisperx">("embedded");
  const [related, setRelated] = useState<RelatedRecord[]>([]);
  const [speakers, setSpeakers] = useState<SpeakerSummary[]>([]);
  const [engineStatus, setEngineStatus] = useState<TranscriptionEngineStatus | null>(null);
  const [correcting, setCorrecting] = useState(false);
  const [exportNotice, setExportNotice] = useState<string | null>(null);
  const [retranscribeLanguage, setRetranscribeLanguage] = useState("zh");
  const [retranscribeModelPath, setRetranscribeModelPath] = useState("");
  const isDocument = currentRecord.sourceType === "document";

  async function load() {
    const requestId = ++loadRequestId.current;
    const isCurrentRequest = () => requestId === loadRequestId.current;
    const corePromise = Promise.allSettled([
      getRecord(record.id),
      listTranscriptBlocks(record.id),
      latestAnalysis(record.id),
    ]);
    const auxiliaryPromise = Promise.allSettled([
      record.sourceType === "document" ? Promise.resolve<string | null>(null) : recordAudioPath(record.id),
      listProjects(),
      listAnalysisTemplates(),
      getLocalAiStatus(),
    ]);

    const [recordResult, blocksResult, analysisResult] = await corePromise;
    if (!isCurrentRequest()) return;

    const coreErrors = [recordResult, blocksResult, analysisResult]
      .filter((result) => result.status === "rejected")
      .map((result) => String(result.reason));
    if (coreErrors.length > 0) {
      setError(`加载记录详情失败：${coreErrors[0]}`);
    }

    const latest = recordResult.status === "fulfilled" ? recordResult.value : null;
    const items = blocksResult.status === "fulfilled" ? blocksResult.value : null;
    const stored = analysisResult.status === "fulfilled" ? analysisResult.value : null;

    if (latest) {
      setCurrentRecord(latest);
      if (!isProcessing(latest.status) && !completionNotified.current) {
        completionNotified.current = true;
        onChanged(latest);
      }
    }
    if (items) {
      setBlocks(items);
      setSegments(items.flatMap((item) => item.segments));
    }
    if (analysisResult.status === "fulfilled") {
      setAnalysis(stored ? parseAnalysis(stored.contentJson) : null);
      if (latest) setSelectedTemplateId(latest.analysisTemplateId ?? stored?.templateId ?? "builtin-standard");
    }

    const [audioResult, projectsResult, templatesResult, localAiResult] = await auxiliaryPromise;
    if (!isCurrentRequest()) return;

    if (audioResult.status === "fulfilled") {
      const path = audioResult.value;
      setAudioLoadState(path ? "loading" : "ready");
      setAudioError("");
      setSource(path ? convertFileSrc(path) : "");
    } else if (record.sourceType !== "document") {
      setAudioLoadState("error");
      setAudioError(`无法载入音频：${String(audioResult.reason)}`);
      setSource("");
    }
    if (projectsResult.status === "fulfilled") setProjects(projectsResult.value);
    if (templatesResult.status === "fulfilled") setTemplates(templatesResult.value);
    if (localAiResult.status === "fulfilled") {
      const localAi = localAiResult.value;
      setAiStatus(localAi);
      setRetranscribeLanguage(localAi.settings.transcriptionLanguage);
      setRetranscribeModelPath(localAi.settings.whisperModelPath || localAi.whisperModelPath || "");
    }
  }

  useEffect(() => {
    setCurrentRecord(record);
    setSource("");
    setAudioLoadState("loading");
    setAudioError("");
    setError("");
    loadRequestId.current += 1;
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
    return () => {
      loadRequestId.current += 1;
    };
  }, [record.id]);

  useEffect(() => {
    if (!isProcessing(currentRecord.status)) return;
    const timer = window.setInterval(() => void load(), 2_000);
    return () => window.clearInterval(timer);
  }, [record.id, currentRecord.status]);

  useEffect(() => {
    if (!navigation) return;
    // 播放/定位不依赖逐字稿：没有 segments 的录音也要能从列表直接播放。
    if (!isDocument && navigation.startMs !== null && audio.current) {
      if (audio.current.readyState >= HTMLMediaElement.HAVE_METADATA) {
        audio.current.currentTime = navigation.startMs / 1000;
      } else {
        pendingSeekMs.current = navigation.startMs;
      }
      setCurrentMs(navigation.startMs);
    }
    // 工作区列表的播放钮走这里：切好起点后直接开播（音频未就绪时交给 pendingPlay）。
    if (navigation.autoPlay && !isDocument && audio.current) {
      if (audio.current.readyState >= HTMLMediaElement.HAVE_METADATA) {
        void audio.current.play().catch((reason) => {
          setAudioLoadState("error");
          setAudioError(`无法播放音频：${String(reason)}`);
        });
      } else {
        pendingPlay.current = true;
      }
    }
    if (segments.length === 0) return;
    setActiveTab("transcript");
    setHighlightedSegmentId(navigation.targetSegmentId);
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
    if (isDocument) return;
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
    if (isDocument) return;
    setBusy(true);
    setError("");
    completionNotified.current = false;
    try {
      await retranscribeRecord(record.id, retranscribeLanguage, retranscribeModelPath || null, "enhanced", retranscribeEngine);
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


  async function exportToOutput(kind: "analysis" | "transcript") {
    if (detailMenu.current) detailMenu.current.open = false;
    setError("");
    try {
      const path = await exportRecordToOutput(currentRecord.id, kind);
      setExportNotice(`已导出：${path}`);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  }

  function seek(ms: number, play = true) {
    if (isDocument) return;
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
    if (Number.isFinite(element.duration)) setDurationMs(element.duration * 1000);
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

  function togglePlay() {
    const element = audio.current;
    if (!element) return;
    setAudioError("");
    if (element.paused) {
      if (element.readyState < HTMLMediaElement.HAVE_METADATA) {
        setAudioLoadState("loading");
        return;
      }
      void element.play().catch((reason) => {
        setAudioLoadState("error");
        setAudioError(`无法播放音频：${String(reason)}`);
      });
    } else {
      element.pause();
    }
  }

  function seekRelative(deltaMs: number) {
    const total = durationMs ?? currentRecord.audioDurationMs;
    seek(Math.min(Math.max(0, currentMs + deltaMs), total), isPlaying);
  }

  function cycleRate() {
    const steps = [1, 1.25, 1.5, 2, 0.75];
    const next = steps[(steps.indexOf(playbackRate) + 1) % steps.length];
    setPlaybackRate(next);
    if (audio.current) audio.current.playbackRate = next;
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
    <section className="detail-pane" aria-label={isDocument ? "文档详情" : "录音详情"}>
      <header className="detail-header">
        <div className="detail-title">
          <span className={`status-dot status-${statusTone(currentRecord.status)}`} aria-hidden="true" />
          <div>
            <p className="pane-eyebrow">{isDocument ? "文档详情" : "录音详情"} · {statusText(currentRecord)}</p>
            {editingTitle ? (
              <form className="title-editor" onSubmit={(event) => { event.preventDefault(); void saveTitle(); }}>
                <input
                  autoFocus
                  value={titleDraft}
                  maxLength={160}
                  aria-label={isDocument ? "文档标题" : "录音标题"}
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
        {exportNotice && <p className="inline-notice" role="status">{exportNotice}</p>}
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
            <div className="detail-menu-popover">
              {!isDocument && <button type="button" disabled={busy || correcting || !currentRecord.hasTranscript} onClick={() => { if (detailMenu.current) detailMenu.current.open = false; void startCorrection(); }}>{correcting ? "AI 校对中…" : "AI 校对逐字稿"}</button>}
              {!isDocument && <button type="button" disabled={busy || isProcessing(currentRecord.status)} onClick={() => { if (detailMenu.current) detailMenu.current.open = false; setRetranscribeOpen(true); }}>增强重新转写…</button>}
              <button type="button" disabled={busy || !currentRecord.hasTranscript} onClick={() => void exportOne("md")}>导出 Markdown</button>
              <button type="button" disabled={busy || !currentRecord.hasTranscript} onClick={() => void exportOne("txt")}>导出 TXT</button>
              <button type="button" disabled={busy || !currentRecord.hasAnalysis} onClick={() => void exportToOutput("analysis")}>导出分析到产出文件夹</button>
              <button type="button" disabled={busy || !currentRecord.hasTranscript} onClick={() => void exportToOutput("transcript")}>导出逐字稿到产出文件夹</button>
            </div>
          </details>
        </div>
      </header>

      {!isDocument && source && (
        <div className="audio-bar">
          {/* 原生控件隐藏，播放/进度/倍速/音量由自定义播放条接管（外观对齐 Lovart 稿） */}
          <audio
            ref={audio}
            preload="metadata"
            src={source}
            onLoadStart={() => {
              setAudioLoadState("loading");
              setDurationMs(null);
              setIsPlaying(false);
            }}
            onLoadedMetadata={(event) => audioReady(event.currentTarget)}
            onCanPlay={(event) => audioReady(event.currentTarget)}
            onError={(event) => audioFailed(event.currentTarget)}
            onTimeUpdate={(event) => setCurrentMs(event.currentTarget.currentTime * 1000)}
            onPlay={() => setIsPlaying(true)}
            onPause={() => setIsPlaying(false)}
            onEnded={() => setIsPlaying(false)}
          />
          <div className="player">
            <div className="player-main">
              <button
                type="button"
                className="player-play"
                aria-label={isPlaying ? "暂停" : "播放"}
                disabled={audioLoadState !== "ready"}
                onClick={togglePlay}
              >
                {isPlaying ? <PauseIcon size={18} /> : <PlayIcon size={18} />}
              </button>
              <button type="button" className="player-skip" aria-label="后退 15 秒" disabled={audioLoadState !== "ready"} onClick={() => seekRelative(-15000)}>
                <SkipBackIcon />
              </button>
              <button type="button" className="player-skip" aria-label="前进 15 秒" disabled={audioLoadState !== "ready"} onClick={() => seekRelative(15000)}>
                <SkipForwardIcon />
              </button>
              <button type="button" className="player-rate" aria-label="播放速度" onClick={cycleRate}>
                {playbackRate}×
              </button>
              <span className="player-time" role="status">
                {formatTime(currentMs)} / {formatTime(durationMs ?? currentRecord.audioDurationMs)}
              </span>
              <span className="player-spacer" />
              <input
                type="range"
                className="player-volume"
                min={0}
                max={1}
                step={0.05}
                value={volume}
                aria-label="音量"
                onChange={(event) => {
                  const next = Number(event.target.value);
                  setVolume(next);
                  if (audio.current) audio.current.volume = next;
                }}
              />
            </div>
            <input
              type="range"
              className="player-seek"
              min={0}
              max={Math.max(1, durationMs ?? currentRecord.audioDurationMs)}
              step={100}
              value={Math.min(currentMs, durationMs ?? currentRecord.audioDurationMs)}
              aria-label="播放进度"
              disabled={audioLoadState !== "ready"}
              onChange={(event) => seek(Number(event.target.value), isPlaying)}
            />
          </div>
          <div className={`audio-status audio-${audioLoadState}`} role="status">
            <span>{audioLoadState === "ready" ? "音频已就绪" : audioLoadState === "loading" ? "正在载入音频" : audioError}</span>
            <span>总时长 {formatTime(currentRecord.audioDurationMs)}</span>
          </div>
        </div>
      )}

      {isProcessing(currentRecord.status) && (
        <div className="processing-progress" role="status">
          <div><strong>{processingStageText(currentRecord.processingStage, isDocument)}</strong><span>{progressText(currentRecord)}</span></div>
          <div className="progress-track"><span style={{ width: `${processingPercent(currentRecord)}%` }} /></div>
        </div>
      )}

      <div className="detail-tabs" role="tablist" aria-label={isDocument ? "文档内容" : "录音内容"}>
        <button type="button" role="tab" aria-selected={activeTab === "transcript"} className={activeTab === "transcript" ? "selected" : ""} onClick={() => setActiveTab("transcript")}>{isDocument ? "文档正文" : "逐字稿"}</button>
        <button type="button" role="tab" aria-selected={activeTab === "analysis"} className={activeTab === "analysis" ? "selected" : ""} onClick={() => setActiveTab("analysis")}>内容分析</button>
      </div>

      {error && <p className="detail-error">{error}</p>}

      <div className="detail-content" ref={detailContent}>
        {activeTab === "transcript" ? (
          segments.length === 0 ? (
            <div className="detail-placeholder">
              <p>{isDocument ? "文档正文为空或解析失败，请重新导入有效文档。" : isProcessing(currentRecord.status) ? "正在本机转写，可继续使用其他功能。" : currentRecord.status === "failed" ? "本地转写失败，音频仍已安全保存。" : "尚未生成逐字稿。"}</p>
              {!isDocument && !isProcessing(currentRecord.status) && <button type="button" className="primary-button" disabled={busy} onClick={() => void startTranscription()}>重新转写</button>}
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
                    documentMode={isDocument}
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
                <strong>{analysis ? "更新内容分析" : "本地内容分析"}</strong>
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
              <p className="analysis-warning">{isDocument ? "文档正文" : "逐字稿"}已变化，当前分析基于旧版本，请重新分析。</p>
            )}
            {currentRecord.lastAnalysisError && !isProcessing(currentRecord.status) && (
              <p className="analysis-error">{currentRecord.lastAnalysisError}</p>
            )}
            {analysis ? <AnalysisView analysis={analysis} documentMode={isDocument} onSeek={(ms, segmentId) => {
              setHighlightedSegmentId(segmentId);
              setActiveTab("transcript");
              window.setTimeout(() => {
                const block = blocks.find((item) => item.segmentIds.includes(segmentId));
                document.getElementById(`block-${block?.id ?? segmentId}`)?.scrollIntoView({ block: "center" });
                if (!isDocument) seek(ms);
              }, 40);
            }} /> : (
              <div className="detail-placeholder"><p>{segments.length ? "还没有内容分析。" : isDocument ? "文档正文可用后即可开始分析。" : "逐字稿完成后即可开始分析。"}</p></div>
            )}
          </div>
        )}
      </div>
      {speakers.length > 0 && !isDocument && (
        <section className="speakers-section" aria-label="说话人">
          <h3>说话人</h3>
          <div className="speakers-list">
            {speakers.map((speaker) => (
              <SpeakerRow
                key={speaker.label}
                recordId={currentRecord.id}
                speaker={speaker}
                onRenamed={() => void load()}
              />
            ))}
          </div>
          {speakers.every((speaker) => speaker.label === "未知") && (
            <p className="speakers-hint">当前没有说话人标注。用 whisperX 引擎重新转写即可自动区分说话人（设置 → 本机 AI → 转写引擎）。</p>
          )}
        </section>
      )}

      {related.length > 0 && (
        <section className="related-records-section" aria-label="相关记录">
          <h3>相关记录</h3>
          <div className="related-records-list">
            {related.map((item) => (
              <button
                type="button"
                key={item.recordId}
                className="related-record-card"
                onClick={() => void (async () => {
                  try {
                    const record = await getRecord(item.recordId);
                    onChanged(record);
                  } catch (reason) {
                    setError(reason instanceof Error ? reason.message : String(reason));
                  }
                })()}
              >
                <strong>{item.title}</strong>
                <span>{Math.round(item.similarity * 100)}% 相关</span>
              </button>
            ))}
          </div>
        </section>
      )}

      {!isDocument && retranscribeOpen && (
        <div className="dialog-scrim" role="presentation" onMouseDown={(event) => event.target === event.currentTarget && setRetranscribeOpen(false)}>
          <section className="retranscribe-dialog" role="dialog" aria-modal="true" aria-label="增强重新转写">
            <header>
              <div><p className="pane-eyebrow">生成新版本</p><h3>增强重新转写</h3></div>
              <button type="button" className="close-button" aria-label="关闭" title="关闭" onClick={() => setRetranscribeOpen(false)}>×</button>
            </header>
            <p className="em-hint">使用音频预处理、智能分块和简体规范化。旧逐字稿会保留，新版本完成后旧分析将标记为需要更新。</p>
            <Field label="转写语言">
              <select className="em-select" value={retranscribeLanguage} onChange={(event) => setRetranscribeLanguage(event.target.value)}>
                {TRANSCRIPTION_LANGUAGES.map(([value, label]) => <option value={value} key={value}>{label}</option>)}
              </select>
            </Field>
            <Field label="转写引擎" hint={retranscribeEngine === "whisperx" ? "whisperX 会标注每位说话人（说话人 1、说话人 2…），完成后可在详情页重命名为真实姓名。" : undefined}>
              <Segmented
                label="转写引擎"
                value={retranscribeEngine}
                onChange={setRetranscribeEngine}
                options={[
                  { value: "embedded", label: "内嵌引擎", hint: "默认，零依赖" },
                  {
                    value: "whisperx",
                    label: engineStatus?.whisperxAvailable ? "whisperX" : "whisperX（未安装）",
                    disabled: !engineStatus?.whisperxAvailable,
                    hint: engineStatus?.whisperxAvailable ? "说话人分离" : "需要先 pip install whisperx",
                  },
                ]}
              />
            </Field>
            {aiStatus && (
              <Field label="Whisper 模型" hint={aiStatus.whisperModelSource ? `当前使用：${aiStatus.whisperModelSource}` : undefined}>
                <List>
                  <ListRow
                    state="done"
                    selected={!retranscribeModelPath}
                    title="使用应用默认模型"
                    meta={aiStatus.whisperModelPath ?? "尚未配置默认模型"}
                    onSelect={() => setRetranscribeModelPath("")}
                  />
                  {aiStatus.whisperModels.map((model) => (
                    <ListRow
                      key={model.path}
                      state="done"
                      selected={retranscribeModelPath === model.path}
                      title={model.id}
                      meta={model.path}
                      trail={`${Math.round(model.size / 1024 / 1024)}MB`}
                      onSelect={() => setRetranscribeModelPath(model.path)}
                    />
                  ))}
                </List>
              </Field>
            )}
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

function AnalysisView({ analysis, documentMode, onSeek }: { analysis: AnalysisContent; documentMode: boolean; onSeek: (ms: number, segmentId: string) => void }) {
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
                const locatable = Boolean(segmentId) && (documentMode || typeof item.start_ms === "number");
                return (
                  <li key={`${name}-${index}`}>
                    <button type="button" disabled={!locatable} onClick={() => locatable && onSeek(item.start_ms ?? 0, segmentId!)}>
                      <strong>{item.text}</strong>
                      <span>{item.quote_text && segmentId ? `“${item.quote_text}”${documentMode ? "" : ` · ${formatTime(item.start_ms ?? 0)}`}` : "无可靠出处"}</span>
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
                const locatable = Boolean(segmentId) && (documentMode || typeof item.start_ms === "number");
                return (
                  <li key={`${section.key}-${index}`}>
                    <button type="button" disabled={!locatable} onClick={() => locatable && onSeek(item.start_ms ?? 0, segmentId!)}>
                      <strong>{item.text}</strong>
                      <span>{item.quote_text && segmentId ? `“${item.quote_text}”${documentMode ? "" : ` · ${formatTime(item.start_ms ?? 0)}`}` : "无可靠出处"}</span>
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

function SegmentRow({ segment, documentMode, active, highlighted, onSeek, onSave }: {
  segment: TranscriptSegment;
  documentMode: boolean;
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
      <span className={`time-button${documentMode ? " document-position" : ""}`} role={documentMode ? undefined : "button"} tabIndex={documentMode ? undefined : 0} onClick={documentMode ? undefined : () => onSeek(segment.startMs)} onKeyDown={documentMode ? undefined : (event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); onSeek(segment.startMs); } }}>{documentMode ? `段落 ${segment.sequence + 1}` : formatTime(segment.startMs)}</span>
      <div>
        <p className="speaker-label">{documentMode ? "正文" : (segment.speakerLabel ?? "说话人未知")}</p>
        <textarea
          value={text}
          onChange={(event) => setText(event.target.value)}
          onBlur={() => {
            if (text !== effective) void onSave(segment, text);
          }}
          aria-label={`${documentMode ? "段落" : "片段"} ${segment.sequence + 1} 文本`}
        />
        {segment.editedText !== null && <small>原文：{segment.originalText}</small>}
      </div>
    </article>
  );
}

function TranscriptBlockRow({ block, documentMode, active, highlightedSegmentId, expanded, onToggle, onSeek, onSave }: {
  block: TranscriptBlock;
  documentMode: boolean;
  active: boolean;
  highlightedSegmentId: string | null;
  expanded: boolean;
  onToggle: () => void;
  onSeek: (ms: number) => void;
  onSave: (segment: TranscriptSegment, text: string) => void;
}) {
  return <article id={`block-${block.id}`} className={`transcript-block${active ? " active" : ""}${highlightedSegmentId ? " search-hit" : ""}`}>
    <div className="transcript-block-header">
      <span className={`time-button${documentMode ? " document-position" : ""}`} role={documentMode ? undefined : "button"} tabIndex={documentMode ? undefined : 0} onClick={documentMode ? undefined : () => onSeek(block.startMs)} onKeyDown={documentMode ? undefined : (event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); onSeek(block.startMs); } }}>{documentMode ? `段落 ${block.segments[0] ? block.segments[0].sequence + 1 : ""}` : formatTime(block.startMs)}</span>
      <span>{documentMode ? "正文" : (block.speakerLabel ?? "说话人未知")}</span>
      <span>{documentMode ? `${block.segments.length} 段` : `${Math.max(1, Math.round((block.endMs - block.startMs) / 1000))} 秒`}</span>
      <button type="button" className="block-edit-button" onClick={onToggle}>{expanded ? "收起" : "编辑"}</button>
    </div>
    {!expanded ? <p>{block.text}</p> : <div className="block-segments">{block.segments.map((segment) => <SegmentRow key={segment.id} segment={segment} active={false} highlighted={segment.id === highlightedSegmentId} documentMode={documentMode} onSeek={onSeek} onSave={onSave} />)}</div>}
  </article>;
}

function parseAnalysis(content: string): AnalysisContent | null {
  try {
    return JSON.parse(content) as AnalysisContent;
  } catch {
    return null;
  }
}

function statusText(record: RecordBrief) {
  if (record.status === "analyzing") return "本地分析中";
  if (record.sourceType === "document" && (record.status === "transcribing" || record.status === "preparing")) return "正在处理文档";
  if (record.status === "transcribing" || record.status === "preparing") return "本地转写中";
  if (record.status === "failed") return record.sourceType === "document" ? "文档处理失败" : "转写失败";
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

function processingStageText(stage: string | null, documentMode = false) {
  if (documentMode) {
    const labels: Record<string, string> = {
      preprocessing: "正在准备文档",
      chunking: "正在整理文档",
      transcribing: "正在处理文档",
      merging: "正在整理文档",
      normalizing: "正在整理文档",
      analyzing: "正在分析文档",
      validating: "正在校验分析引用",
      saving: "正在保存分析结果",
    };
    return stage ? labels[stage] ?? "正在处理文档" : "正在处理文档";
  }

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

function SpeakerRow({ recordId, speaker, onRenamed }: { recordId: string; speaker: SpeakerSummary; onRenamed: () => void }) {
  const [name, setName] = useState(speaker.label);
  const [busy, setBusy] = useState(false);
  const [addHotword, setAddHotword] = useState(false);
  const [error, setError] = useState("");

  async function rename() {
    const next = name.trim();
    if (!next || next === speaker.label || busy) return;
    setBusy(true);
    setError("");
    try {
      await renameRecordSpeaker(recordId, speaker.label, next, addHotword);
      onRenamed();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="speaker-row">
      <input
        value={name}
        aria-label={`说话人 ${speaker.label} 名称`}
        onChange={(event) => setName(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.nativeEvent.isComposing) void rename();
        }}
      />
      <span className="speaker-count">{speaker.segmentCount} 段</span>
      <label className="speaker-hotword">
        <input type="checkbox" checked={addHotword} onChange={(event) => setAddHotword(event.target.checked)} />
        同时加入词汇库
      </label>
      <button type="button" className="toolbar-button" disabled={busy || !name.trim() || name.trim() === speaker.label} onClick={() => void rename()}>
        {busy ? "保存中…" : "重命名"}
      </button>
      {error && <small className="inline-error">{error}</small>}
    </div>
  );
}
