import { useEffect, useState } from "react";
import ProjectPanel from "./components/ProjectPanel";
import ActionDashboard from "./components/ActionDashboard";
import OnboardingWizard from "./components/OnboardingWizard";
import AppToolbar, { type MainView } from "./components/AppToolbar";
import AssistantDock from "./components/AssistantDock";
import RecordPanel from "./components/RecordPanel";
import RecordDetail from "./components/RecordDetail";
import KnowledgeHome from "./components/KnowledgeHome";
import KnowledgeChatView from "./components/KnowledgeChatView";
import GrowthView from "./components/GrowthView";
import EvolutionView from "./components/EvolutionView";
import SettingsPanel from "./components/SettingsPanel";
import type { KnowledgeAnswerCitation, MemoryScope, MemorySourceReference, RecordBrief, SearchResult } from "./shared/types";
import { getOnboardingStatus, getRecord } from "./lib/tauri";

export interface RecordNavigation {
  token: number;
  startMs: number | null;
  targetSegmentId: string | null;
  /** 工作区列表播放钮使用：定位后直接开始播放。 */
  autoPlay?: boolean;
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
      setMainView("library");
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

  /** 工作区列表的播放钮：选中记录、切回首页并自动开播。 */
  function playFromList(record: RecordBrief) {
    setSelectedRecord(record);
    setMainView("library");
    setNavigation({ token: Date.now(), startMs: 0, targetSegmentId: null, autoPlay: true });
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
      <AppToolbar
        mainView={mainView}
        onSelectView={setMainView}
        projectId={projectId}
        unfiledOnly={unfiledOnly}
        onOpenSearchResult={(result) => void openSearchResult(result)}
        onOpenSettings={() => setSettingsOpen(true)}
      />

      <AssistantDock
        selectedRecordId={selectedRecord?.id ?? null}
        selectedRecordTitle={selectedRecord?.title ?? null}
      />

      <div className={mainView === "library" ? "workspace-grid" : `memory-workspace-grid${mainView === "actions" ? " solo" : ""}`}>
        {/* 行动页不消费知识库范围，是唯一不放侧栏的视图；其余四个视图侧栏都接了真实数据 */}
        {mainView !== "actions" && <ProjectPanel selectedScope={scope} refreshKey={refreshKey} onSelect={selectScope} />}
        {mainView === "library" ? <>
          {/* 不用 key 强制重挂载：那会在批量移动/删除后把面板里的结果提示一并清掉，
              用户看不到操作是否成功。范围切换时的状态重置由面板自己的 effect 负责。 */}
          <RecordPanel
            projectId={projectId}
            unfiledOnly={unfiledOnly}
            onImported={() => changed()}
            selectedId={selectedRecord?.id ?? null}
            onSelect={setSelectedRecord}
            onPlay={playFromList}
          />
          {selectedRecord ? (
            <RecordDetail record={selectedRecord} navigation={navigation} onChanged={changed} />
          ) : (
            <KnowledgeHome scope={scope} projectId={projectId} unfiledOnly={unfiledOnly} refreshKey={refreshKey} onOpenCitation={(citation) => void openCitation(citation)} onOpenKnowledgeChat={() => setMainView("chat")} />
          )}
        </> : <div className="memory-main-content">
          {mainView === "chat" && <KnowledgeChatView scope={scope} projectId={projectId} unfiledOnly={unfiledOnly} refreshKey={refreshKey} onOpenCitation={(citation) => void openCitation(citation)} onOpenSettings={() => setSettingsOpen(true)} />}
          {mainView === "growth" && <GrowthView scope={memoryScope} onOpenSource={(source) => void openMemorySource(source)} onOpenSettings={() => setSettingsOpen(true)} />}
          {mainView === "evolution" && <EvolutionView scope={memoryScope} onOpenSource={(source) => void openMemorySource(source)} onOpenSettings={() => setSettingsOpen(true)} />}
          {mainView === "actions" && <ActionDashboard onOpenRecord={openActionRecord} />}
        </div>}
      </div>
      <SettingsPanel
        open={settingsOpen}
        onClose={() => { setSettingsOpen(false); changed(); }}
        onOpenKnowledge={() => setMainView("library")}
      />
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
