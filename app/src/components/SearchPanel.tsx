import { Fragment, useEffect, useRef, useState } from "react";
import type { SearchResult } from "../shared/types";
import { searchRecords } from "../lib/tauri";

interface Props {
  projectId: string | null;
  unfiledOnly: boolean;
  onOpen: (result: SearchResult) => void;
}

const SOURCE_LABEL: Record<SearchResult["sourceType"], string> = {
  title: "标题",
  transcript: "逐字稿",
  analysis: "文稿分析",
};

export default function SearchPanel({ projectId, unfiledOnly, onOpen }: Props) {
  const root = useRef<HTMLElement>(null);
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [error, setError] = useState("");
  const [searched, setSearched] = useState(false);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    function onPointerDown(event: PointerEvent) {
      if (root.current && !root.current.contains(event.target as Node)) setOpen(false);
    }
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") setOpen(false);
    }
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, []);

  useEffect(() => {
    setResults([]);
    setSearched(false);
    setOpen(false);
  }, [projectId, unfiledOnly]);

  async function search() {
    const trimmed = query.trim();
    if (!trimmed) {
      setResults([]);
      setSearched(false);
      setOpen(false);
      return;
    }
    try {
      setError("");
      setResults(await searchRecords(trimmed, projectId, unfiledOnly));
    } catch (reason) {
      setError(String(reason));
      setResults([]);
    } finally {
      setSearched(true);
      setOpen(true);
    }
  }

  function clear() {
    setQuery("");
    setResults([]);
    setError("");
    setSearched(false);
    setOpen(false);
  }

  return (
    <section className="search-panel" ref={root}>
      <div className="search-form">
        <div className="search-input-wrap">
          <input
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              if (!event.target.value) clear();
            }}
            onFocus={() => searched && setOpen(true)}
            onKeyDown={(event) => event.key === "Enter" && void search()}
            placeholder="搜索录音、逐字稿和分析"
            aria-label="搜索记录"
          />
          {query && (
            <button type="button" className="clear-button" onClick={clear} aria-label="清空搜索" title="清空搜索">
              ×
            </button>
          )}
        </div>
        <button type="button" className="primary-button compact-button" onClick={() => void search()}>
          搜索
        </button>
      </div>

      {open && (
        <div className="search-popover material" role="dialog" aria-label="搜索结果">
          <div className="popover-header">
            <strong>搜索结果</strong>
            <span>{results.length} 条</span>
          </div>
          <div className="search-results">
            {results.map((result) => (
              <button
                key={`${result.recordId}-${result.sourceId}`}
                type="button"
                className="search-result"
                onClick={() => {
                  setOpen(false);
                  onOpen(result);
                }}
              >
                <span className="result-title-row">
                  <strong>{result.title}</strong>
                  <span className="source-label">{SOURCE_LABEL[result.sourceType]}</span>
                </span>
                <span className="result-snippet">{highlightSnippet(result.snippet)}</span>
                <span className="result-meta">
                  {new Date(result.importedAt).toLocaleDateString()} · {result.projectName ?? "未归档"} · {result.speakerLabel ?? "说话人未知"}
                  {result.startMs !== null ? ` · ${formatTime(result.startMs)}` : ""}
                </span>
              </button>
            ))}
            {searched && !error && results.length === 0 && <p className="popover-empty">没有找到匹配内容。</p>}
            {error && <p className="inline-error">{error}</p>}
          </div>
        </div>
      )}
    </section>
  );
}

function highlightSnippet(snippet: string) {
  return snippet.split(/(<mark>|<\/mark>)/).map((part, index, parts) => {
    if (part === "<mark>" || part === "</mark>") return null;
    const highlighted = parts[index - 1] === "<mark>";
    return highlighted ? <mark key={index}>{part}</mark> : <Fragment key={index}>{part}</Fragment>;
  });
}

function formatTime(ms: number) {
  const seconds = Math.floor(ms / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}
