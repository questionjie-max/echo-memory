import { useEffect, useState } from "react";
import ProjectPanel from "./components/ProjectPanel";
import ActionDashboard from "./components/ActionDashboard";
import OnboardingWizard from "./components/OnboardingWizard";
import RecordPanel from "./components/RecordPanel";
import RecordDetail from "./components/RecordDetail";
import SearchPanel from "./components/SearchPanel";
import KnowledgeHome from "./components/KnowledgeHome";
import KnowledgeChatView from "./components/KnowledgeChatView";
import GrowthView from "./components/GrowthView";
import EvolutionView from "./components/EvolutionView";
import SettingsPanel from "./components/SettingsPanel";
import type { KnowledgeAnswerCitation, MemoryScope, MemorySourceReference, RecordBrief, SearchResult } from "./shared/types";
import { getOnboardingStatus, getRecord } from "./lib/tauri";

type MainView = "library" | "chat" | "growth" | "evolution" | "actions";

export interface RecordNavigation {
  token: number;
  startMs: number | null;
  targetSegmentId: string | null;
}

export default function App() {
  const [scope, setScope] = useState("all");
  const [mainView, setMainView] = useState<MainView>("library");
  const [refreshKey, setRefreshKey] = useState(0);
  const [selectedRecord, setSelectedRecord] = useState<RecordBrief | null>(null);
  const [navigation, setNavigation] = useState<RecordNavigation | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [onboardingNeeded, setOnboardingNeeded] = useState<boolean | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const status = await getOnboardingStatus();
        if (!cancelled) setOnboardingNeeded(!status.completed);
      } catch {
        if (!cancelled) setOnboardingNeeded(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const projectId = scope === "all" || scope === "unfiled" ? null : scope;
  const unfiledOnly = scope === "unfiled";

  async function openSearchResult(result: SearchResult) {
    try {
      const record = await getRecord(result.recordId);
      setSelectedRecord(record);
      setNavigation({
        token: Date.now(),
        startMs: result.startMs,
        targetSegmentId: result.targetSegmentId,
      });
    } catch (reason) {
      const detail = reason instanceof Error ? reason.message : String(reason);
      window.alert(`无法打开搜索结果。该记录可能已被删除。${detail ? `\n\n${detail}` : ""}`);
    }
  }

  async function openCitation(citation: KnowledgeAnswerCitation) {
    try {
      const record = await getRecord(citation.recordId);
      setSelectedRecord(record);
      setNavigation({ token: Date.now(), startMs: citation.startMs, targetSegmentId: citation.segmentId });
      setMainView("library");
    } catch (reason) {
      const detail = reason instanceof Error ? reason.message : String(reason);
      window.alert(`无法打开引用原文。该记录可能已被删除或索引已经过期。${detail ? `\n\n${detail}` : ""}`);
    }
  }

  async function openMemorySource(source: MemorySourceReference) {
    try {
      const record = await getRecord(source.recordId);
      setSelectedRecord(record);
      setNavigation({ token: Date.now(), startMs: source.startMs, targetSegmentId: source.segmentId });
      setMainView("library");
    } catch (reason) {
      const detail = reason instanceof Error ? reason.message : String(reason);
      window.alert(`无法打开成长轨迹原文。该记录可能已被删除。${detail ? `\n\n${detail}` : ""}`);
    }
  }

  async function openActionRecord(recordId: string, segmentId: string | null) {
    try {
      const record = await getRecord(recordId);
      setSelectedRecord(record);
      setNavigation({ token: Date.now(), startMs: null, targetSegmentId: segmentId });
      setMainView("library");
    } catch (reason) {
      const detail = reason instanceof Error ? reason.message : String(reason);
      window.alert(`无法打开该记录。它可能已被删除。${detail ? `\n\n${detail}` : ""}`);
    }
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

  const memoryScope: MemoryScope = scope === "unfiled"
    ? { kind: "unfiled", projectId: null }
    : scope === "all"
      ? { kind: "all", projectId: null }
      : { kind: "project", projectId: scope };

  return (
    <main className="app-shell">
      <header className="app-toolbar material">
        <div className="brand-block">
          <h1>回声记忆</h1>
          <p>默认本地处理；启用外部 AI 后仅发送所选文本，永不上传音频</p>
        </div>
        <nav className="main-navigation" aria-label="主视图">
          <NavButton label="资料库" selected={mainView === "library"} onClick={() => setMainView("library")} />
          <NavButton label="AI 对话" selected={mainView === "chat"} onClick={() => setMainView("chat")} />
          <NavButton label="成长轨迹" selected={mainView === "growth"} onClick={() => setMainView("growth")} />
          <NavButton label="认知演化" selected={mainView === "evolution"} onClick={() => setMainView("evolution")} />
          <NavButton label="行动" selected={mainView === "actions"} onClick={() => setMainView("actions")} />
        </nav>
        {mainView === "library" && <SearchPanel
          projectId={projectId}
          unfiledOnly={unfiledOnly}
          onOpen={(result) => void openSearchResult(result)}
        />}
        <button type="button" className="toolbar-button" onClick={() => setSettingsOpen(true)}>设置</button>
      </header>

      <div className={mainView === "library" ? "workspace-grid" : "memory-workspace-grid"}>
        <ProjectPanel selectedScope={scope} refreshKey={refreshKey} onSelect={selectScope} />
        {mainView === "library" ? <>
          <RecordPanel
            key={`${scope}-${refreshKey}`}
            projectId={projectId}
            unfiledOnly={unfiledOnly}
            onImported={() => changed()}
            selectedId={selectedRecord?.id ?? null}
            onSelect={setSelectedRecord}
          />
          {selectedRecord ? (
            <RecordDetail record={selectedRecord} navigation={navigation} onChanged={changed} />
          ) : (
            <KnowledgeHome scope={scope} projectId={projectId} unfiledOnly={unfiledOnly} refreshKey={refreshKey} onOpenCitation={(citation) => void openCitation(citation)} />
          )}
        </> : <div className="memory-main-content">
          {mainView === "chat" && <KnowledgeChatView scope={scope} projectId={projectId} unfiledOnly={unfiledOnly} refreshKey={refreshKey} onOpenCitation={(citation) => void openCitation(citation)} onOpenSettings={() => setSettingsOpen(true)} />}
          {mainView === "growth" && <GrowthView scope={memoryScope} onOpenSource={(source) => void openMemorySource(source)} onOpenSettings={() => setSettingsOpen(true)} />}
          {mainView === "evolution" && <EvolutionView scope={memoryScope} onOpenSource={(source) => void openMemorySource(source)} onOpenSettings={() => setSettingsOpen(true)} />}
          {mainView === "actions" && <ActionDashboard onOpenRecord={openActionRecord} />}
        </div>}
      </div>
      <SettingsPanel open={settingsOpen} onClose={() => { setSettingsOpen(false); changed(); }} />
      {onboardingNeeded && (
        <OnboardingWizard
          onFinished={() => {
            setOnboardingNeeded(false);
            changed();
          }}
        />
      )}
    </main>
  );
}

function NavButton({ label, selected, onClick }: { label: string; selected: boolean; onClick: () => void }) {
  return <button type="button" className={`main-navigation-button${selected ? " selected" : ""}`} aria-current={selected ? "page" : undefined} onClick={onClick}>{label}</button>;
}
