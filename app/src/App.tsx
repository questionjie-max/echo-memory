import { useState } from "react";
import ProjectPanel from "./components/ProjectPanel";
import RecordPanel from "./components/RecordPanel";
import RecordDetail from "./components/RecordDetail";
import SearchPanel from "./components/SearchPanel";
import KnowledgeHome from "./components/KnowledgeHome";
import SettingsPanel from "./components/SettingsPanel";
import type { KnowledgeAnswerCitation, RecordBrief, SearchResult } from "./shared/types";
import { getRecord } from "./lib/tauri";

export interface RecordNavigation {
  token: number;
  startMs: number | null;
  targetSegmentId: string | null;
}

export default function App() {
  const [scope, setScope] = useState("all");
  const [refreshKey, setRefreshKey] = useState(0);
  const [selectedRecord, setSelectedRecord] = useState<RecordBrief | null>(null);
  const [navigation, setNavigation] = useState<RecordNavigation | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);

  const projectId = scope === "all" || scope === "unfiled" ? null : scope;
  const unfiledOnly = scope === "unfiled";

  async function openSearchResult(result: SearchResult) {
    const record = await getRecord(result.recordId);
    setSelectedRecord(record);
    setNavigation({
      token: Date.now(),
      startMs: result.startMs,
      targetSegmentId: result.targetSegmentId,
    });
  }

  async function openCitation(citation: KnowledgeAnswerCitation) {
    const record = await getRecord(citation.recordId);
    setSelectedRecord(record);
    setNavigation({ token: Date.now(), startMs: citation.startMs, targetSegmentId: citation.segmentId });
  }

  function selectScope(nextScope: string) {
    setScope(nextScope);
    setSelectedRecord(null);
    setNavigation(null);
  }

  function changed(record?: RecordBrief) {
    if (record) setSelectedRecord(record);
    setRefreshKey((value) => value + 1);
  }

  return (
    <main className="app-shell">
      <header className="app-toolbar material">
        <div className="brand-block">
          <h1>回声记忆</h1>
          <p>本地处理，音频和逐字稿不离开本机</p>
        </div>
        <SearchPanel
          projectId={projectId}
          unfiledOnly={unfiledOnly}
          onOpen={(result) => void openSearchResult(result)}
        />
        <button type="button" className="toolbar-button" onClick={() => setSettingsOpen(true)}>设置</button>
      </header>

      <div className="workspace-grid">
        <ProjectPanel
          selectedScope={scope}
          refreshKey={refreshKey}
          onSelect={selectScope}
        />
        <RecordPanel
          key={`${scope}-${refreshKey}`}
          projectId={projectId}
          unfiledOnly={unfiledOnly}
          onImported={() => changed()}
          selectedId={selectedRecord?.id ?? null}
          onSelect={setSelectedRecord}
        />
        {selectedRecord ? (
          <RecordDetail
            record={selectedRecord}
            navigation={navigation}
            onChanged={changed}
          />
        ) : (
          <KnowledgeHome
            scope={scope}
            projectId={projectId}
            unfiledOnly={unfiledOnly}
            refreshKey={refreshKey}
            onOpenCitation={(citation) => void openCitation(citation)}
          />
        )}
      </div>
      <SettingsPanel open={settingsOpen} onClose={() => { setSettingsOpen(false); changed(); }} />
    </main>
  );
}
