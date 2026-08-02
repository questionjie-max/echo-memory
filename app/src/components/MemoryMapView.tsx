import { useEffect, useMemo, useRef, useState } from "react";
import MemoryViewControls from "./MemoryViewControls";
import type { ExternalAiSettings, MemoryEdge, MemoryNode, MemoryScope, MemorySourceReference } from "../shared/types";
import { useMemoryViewData, type MemoryRangeKey } from "../hooks/useMemoryViewData";

interface Props {
  scope: MemoryScope;
  onOpenSource: (source: MemorySourceReference) => void;
  onOpenSettings: () => void;
}

const TYPE_LABELS: Record<string, string> = {
  project: "项目",
  event: "事件",
  theme: "主题",
  viewpoint: "观点",
  decision: "决策",
  task: "任务",
  person: "人物",
};

const TYPE_COLORS: Record<string, string> = {
  project: "#007aff",
  event: "#34c759",
  theme: "#af52de",
  viewpoint: "#ff9f0a",
  decision: "#ff375f",
  task: "#5e5ce6",
  person: "#8e8e93",
};

export default function MemoryMapView({ scope, onOpenSource, onOpenSettings }: Props) {
  const [range, setRange] = useState<MemoryRangeKey>("30d");
  const [filter, setFilter] = useState<string>("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const drag = useRef<{ x: number; y: number; panX: number; panY: number } | null>(null);
  const data = useMemoryViewData("map", scope, range);
  const snapshot = data.selectedSnapshot;
  const nodes = snapshot?.result.nodes ?? [];
  const visibleNodes = useMemo(() => filter === "all" ? nodes : nodes.filter((node) => node.nodeType === filter), [filter, nodes]);
  const visibleIds = new Set(visibleNodes.map((node) => node.id));
  const edges = (snapshot?.result.edges ?? []).filter((edge) => visibleIds.has(edge.sourceId) && visibleIds.has(edge.targetId));
  const positions = useMemo(() => layoutNodes(visibleNodes), [visibleNodes]);
  const selected = visibleNodes.find((node) => node.id === selectedId) ?? null;
  const types = Array.from(new Set(nodes.map((node) => node.nodeType)));

  useEffect(() => {
    setFilter("all");
    setSelectedId(null);
    setZoom(1);
    setPan({ x: 0, y: 0 });
    drag.current = null;
  }, [scope.kind, scope.projectId, range, data.selectedSnapshot?.id]);

  function onPointerDown(event: React.PointerEvent<SVGSVGElement>) {
    drag.current = { x: event.clientX, y: event.clientY, panX: pan.x, panY: pan.y };
    event.currentTarget.setPointerCapture(event.pointerId);
  }
  function onPointerMove(event: React.PointerEvent<SVGSVGElement>) {
    if (!drag.current) return;
    setPan({ x: drag.current.panX + event.clientX - drag.current.x, y: drag.current.panY + event.clientY - drag.current.y });
  }
  function onPointerUp() { drag.current = null; }
  function onWheel(event: React.WheelEvent<SVGSVGElement>) {
    event.preventDefault();
    setZoom((value) => clamp(value - event.deltaY * 0.001, 0.55, 1.8));
  }

  return <section className="memory-view map-view">
    <MemoryViewControls
      title="记忆地图"
      description="用确定性分层布局查看项目、事件、主题、观点、决策和任务之间的关系。"
      viewKind="map"
      scope={scope}
      range={range}
      onRangeChange={setRange}
      settings={data.settings}
      snapshots={data.snapshots}
      selectedSnapshotId={data.selectedSnapshotId}
      onSelectSnapshot={data.setSelectedSnapshotId}
      sourceRecordCount={data.sourceRecordCount}
      generating={data.generating}
      onGenerate={data.generate}
      onCancel={data.cancel}
      onOpenSettings={onOpenSettings}
    />
    {data.loading ? (
      <div className="memory-empty" role="status" aria-live="polite"><strong>正在加载记忆地图…</strong></div>
    ) : data.error ? (
      <div className="memory-empty memory-error-state" role="alert"><strong>记忆地图加载失败</strong><p>{data.error}</p><button type="button" className="secondary-button" onClick={() => void data.reload()}>重试</button></div>
    ) : !data.selectedSnapshot ? (
      data.settings && !isExternalAiReady(data.settings) ? <ConfiguredEmpty onOpenSettings={onOpenSettings} /> : <NoSnapshotEmpty sourceRecordCount={data.sourceRecordCount} />
    ) : <>
      <div className="map-toolbar">
        <label>节点筛选<select value={filter} onChange={(event) => { setFilter(event.target.value); setSelectedId(null); }}><option value="all">全部类型</option>{types.map((type) => <option value={type} key={type}>{TYPE_LABELS[type] ?? type}</option>)}</select></label>
        <span role="status" aria-live="polite" aria-atomic="true">{selected ? `已选择${TYPE_LABELS[selected.nodeType] ?? selected.nodeType}“${selected.label}”` : `${visibleNodes.length} 个节点 · ${edges.length} 条关系`}</span>
        <div className="map-zoom"><button type="button" onClick={() => setZoom((value) => clamp(value - 0.1, 0.55, 1.8))}>−</button><span>{Math.round(zoom * 100)}%</span><button type="button" onClick={() => setZoom((value) => clamp(value + 0.1, 0.55, 1.8))}>＋</button><button type="button" onClick={() => { setZoom(1); setPan({ x: 0, y: 0 }); }}>重置</button></div>
      </div>
      <div className="map-layout">
        <div className="map-canvas" aria-label="记忆关系地图">
          {visibleNodes.length === 0 ? <div className="memory-empty"><strong>当前快照没有可展示节点</strong><p>可以调整筛选，或重新生成快照。</p></div> : <svg viewBox="0 0 1000 620" role="img" onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerUp={onPointerUp} onPointerCancel={onPointerUp} onWheel={onWheel}>
            <g transform={`translate(${pan.x} ${pan.y}) scale(${zoom})`}>
              {edges.map((edge) => <MapEdge key={edge.id} edge={edge} source={positions.get(edge.sourceId)} target={positions.get(edge.targetId)} />)}
              {visibleNodes.map((node) => <MapNode node={node} point={positions.get(node.id)!} selected={selectedId === node.id} onClick={() => setSelectedId(node.id)} key={node.id} />)}
            </g>
          </svg>}
        </div>
        <MapDetails node={selected} onOpenSource={onOpenSource} />
      </div>
    </>}
  </section>;
}

function NoSnapshotEmpty({ sourceRecordCount }: { sourceRecordCount: number }) {
  return <div className="memory-config-empty"><div className="memory-config-icon">⌘</div><h3>还没有记忆地图快照</h3><p>{sourceRecordCount ? "生成一份快照后，这里会展示记录之间的关系。" : "当前范围还没有可分析的记录，请先导入并完成转写。"}</p></div>;
}

function isExternalAiReady(settings: ExternalAiSettings) {
  return settings.enabled && settings.hasApiKey && Boolean(settings.privacyConsentAt);
}

function ConfiguredEmpty({ onOpenSettings }: { onOpenSettings: () => void }) {
  return <div className="memory-config-empty"><div className="memory-config-icon">⌘</div><h3>记忆地图需要外部 AI</h3><p>地图关系由你配置的 OpenAI-compatible 服务生成。原始音频不会上传，生成前会再次显示发送范围。</p><button type="button" className="primary-button" onClick={onOpenSettings}>打开外部 AI 设置</button></div>;
}

function MapNode({ node, point, selected, onClick }: { node: MemoryNode; point: Point; selected: boolean; onClick: () => void }) {
  const color = TYPE_COLORS[node.nodeType] ?? "#007aff";
  return <g className={`map-node${selected ? " selected" : ""}`} transform={`translate(${point.x} ${point.y})`} onClick={onClick} onKeyDown={(event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onClick();
    }
  }} tabIndex={0} role="button" aria-pressed={selected} aria-label={`${TYPE_LABELS[node.nodeType] ?? node.nodeType}: ${node.label}`}>
    <circle r={selected ? 25 : 21} fill="#fff" stroke={color} strokeWidth={selected ? 4 : 2} />
    <circle r="5" fill={color} />
    <text y="38" textAnchor="middle">{truncate(node.label, 18)}</text>
    <text className="map-node-type" y="52" textAnchor="middle">{TYPE_LABELS[node.nodeType] ?? node.nodeType}</text>
  </g>;
}

function MapEdge({ edge, source, target }: { edge: MemoryEdge; source?: Point; target?: Point }) {
  if (!source || !target) return null;
  return <g className="map-edge"><line x1={source.x} y1={source.y} x2={target.x} y2={target.y} /><text x={(source.x + target.x) / 2} y={(source.y + target.y) / 2 - 5} textAnchor="middle">{edge.relation}</text></g>;
}

function MapDetails({ node, onOpenSource }: { node: MemoryNode | null; onOpenSource: (source: MemorySourceReference) => void }) {
  if (!node) return <aside className="map-details memory-empty"><strong>选择一个节点</strong><p>点击节点查看摘要、置信度和来源。</p></aside>;
  return <aside className="map-details"><div className="map-details-heading"><span className="memory-type-badge">{TYPE_LABELS[node.nodeType] ?? node.nodeType}</span>{node.inferred && <span className="inferred-badge">模型推断</span>}</div><h3>{node.label}</h3><p>{node.summary || "暂无摘要"}</p>{node.confidence !== null && <span className="confidence-label">置信度 {Math.round(node.confidence * 100)}%</span>}<div className="source-list">{node.sources.length ? node.sources.map((source, index) => <button type="button" className="source-reference" key={`${source.recordId}-${source.segmentId}-${index}`} onClick={() => onOpenSource(source)}><span>来源 {index + 1}</span><span>{source.quoteText || "跳转到逐字稿"}</span></button>) : <span className="settings-meta">没有可匹配的来源</span>}</div></aside>;
}

type Point = { x: number; y: number };
function layoutNodes(nodes: MemoryNode[]) {
  const byType = new Map<string, MemoryNode[]>();
  for (const node of nodes) byType.set(node.nodeType, [...(byType.get(node.nodeType) ?? []), node]);
  const types = Array.from(byType.keys()).sort();
  const positions = new Map<string, Point>();
  const width = 820;
  const top = 90;
  const rowHeight = types.length > 1 ? 450 / (types.length - 1) : 0;
  types.forEach((type, row) => {
    const rowNodes = byType.get(type)!;
    rowNodes.forEach((node, index) => {
      const x = rowNodes.length === 1 ? width / 2 + 90 : 90 + (width - 180) * (index / (rowNodes.length - 1));
      positions.set(node.id, { x, y: top + row * rowHeight });
    });
  });
  return positions;
}
function clamp(value: number, min: number, max: number) { return Math.max(min, Math.min(max, value)); }
function truncate(value: string, length: number) { return value.length > length ? `${value.slice(0, length - 1)}…` : value; }
