/**
 * 顶栏：品牌 + 主导航 + 搜索 + 设置。
 *
 * 版式契约是「四个固定角色」：品牌可压缩、导航不收缩、搜索吃剩余宽度、设置不收缩。
 * 关键约束是搜索框必须**常驻** —— 它是顶栏里唯一带 flex-grow 的元素，
 * 一旦按视图增减，justify-content: space-between 会把剩余宽度重新分配，
 * 导航就会横着挪（这正是此前「导航来回跳」的成因）。
 */
import SearchPanel from "./SearchPanel";
import { PRIVACY_POSTURE } from "../lib/posture";

export type MainView = "library" | "chat" | "growth" | "evolution" | "actions";

const NAV_ITEMS: Array<{ id: MainView; label: string }> = [
  { id: "library", label: "首页" },
  { id: "chat", label: "问知识库" },
  { id: "growth", label: "成长轨迹" },
  { id: "evolution", label: "认知演化" },
  { id: "actions", label: "行动" },
];

export default function AppToolbar({
  mainView,
  onSelectView,
  projectId,
  unfiledOnly,
  onOpenSearchResult,
  onOpenSettings,
}: {
  mainView: MainView;
  onSelectView: (view: MainView) => void;
  projectId: string | null;
  unfiledOnly: boolean;
  onOpenSearchResult: (result: import("../shared/types").SearchResult) => void;
  onOpenSettings: () => void;
}) {
  return (
    <header className="app-toolbar">
      <div className="brand-block">
        <h1>回声记忆</h1>
        <p>{PRIVACY_POSTURE.toolbar}</p>
      </div>
      <nav className="main-navigation" aria-label="主视图">
        {NAV_ITEMS.map((item) => (
          <button
            key={item.id}
            type="button"
            className="main-navigation-button"
            aria-current={mainView === item.id ? "page" : undefined}
            onClick={() => onSelectView(item.id)}
          >
            {item.label}
          </button>
        ))}
      </nav>
      <SearchPanel projectId={projectId} unfiledOnly={unfiledOnly} onOpen={onOpenSearchResult} />
      <button type="button" className="toolbar-button" onClick={onOpenSettings}>
        设置
      </button>
    </header>
  );
}
