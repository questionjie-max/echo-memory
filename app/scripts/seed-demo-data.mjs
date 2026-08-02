#!/usr/bin/env node
import { existsSync, mkdirSync, openSync, closeSync, writeSync, ftruncateSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { homedir } from "node:os";
import { spawnSync } from "node:child_process";

const defaultRoot = process.env.ECHO_LIBRARY_ROOT || join(homedir(), "Library", "Application Support", "回声记忆");
const dbPath = resolve(process.argv[2] || join(defaultRoot, "memory.db"));
const libraryRoot = dirname(dbPath);

if (!existsSync(dbPath)) {
  console.error(`数据库不存在：${dbPath}`);
  process.exit(1);
}

function runSql(input) {
  const result = spawnSync("sqlite3", ["-bail", dbPath], { input, encoding: "utf8", maxBuffer: 20 * 1024 * 1024 });
  if (result.status !== 0) {
    console.error(result.stderr || result.stdout);
    process.exit(result.status || 1);
  }
  return result.stdout.trim();
}

if (runSql("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memory_snapshots';") !== "1") {
  console.error("数据库尚未包含 memory_snapshots 表，请先启动一次最新版应用完成迁移。");
  process.exit(1);
}

const projects = [
  { id: "demo-project-product", name: "产品研发", description: "产品策略、体验优化、技术方案与版本发布" },
  { id: "demo-project-growth", name: "客户与增长", description: "客户访谈、增长实验、定价和商业化" },
  { id: "demo-project-team", name: "团队管理", description: "周会、招聘、协作机制与人才发展" },
  { id: "demo-project-learning", name: "学习与思考", description: "读书、课程、行业研究与个人方法论" },
  { id: "demo-project-life", name: "生活规划", description: "健康、旅行、家庭安排与个人计划" },
];

const records = [
  rec("01", "demo-project-learning", 1, "AI 产品设计课程复盘", "学习方法", "课程强调先定义用户决策，再选择 AI 能力，而不是从模型功能倒推产品。", "原型验证应聚焦用户是否愿意交出关键决策，而不只是点击率。", "课程案例表明可解释反馈会显著提升用户信任。", "下一轮学习笔记统一采用“场景—决策—证据”结构。", "整理三篇课程案例并补充到团队分享", "小林", "本周五", "用什么指标衡量用户真正信任 AI 建议？", "补充"),
  rec("02", "demo-project-product", 2, "移动端语音记录灰度发布评审", "发布策略", "团队评审了语音记录新版的灰度发布范围、回滚条件和观测指标。", "首批灰度只覆盖内部用户和 5% 新用户。", "崩溃率、转写完成率和次日回访是首轮核心指标。", "按 5%—20%—50% 三阶段灰度，每阶段至少观察 48 小时。", "配置灰度开关与异常自动回滚", "阿杰", "8 月 3 日", "弱网环境下的上传失败率是否会成为主要风险？", "修正"),
  rec("03", "demo-project-life", 3, "九月家庭旅行计划讨论", "旅行规划", "家庭初步比较了成都、泉州和大理三条路线，重点考虑老人步行强度。", "每天连续步行最好控制在两小时以内。", "住宿位置比景点数量更影响整体体验。", "优先选择泉州五天四晚方案，并保留一天完全自由活动。", "对比三家市中心酒店的无障碍条件", "我", "本周日", "国庆前机票价格是否还会有明显上涨？", "新增"),
  rec("04", "demo-project-growth", 4, "重点客户流失预警访谈", "客户留存", "销售与客户成功复盘了三家重点客户近期活跃下降的原因。", "客户流失信号通常先出现在核心使用人的登录频率下降。", "只看合同到期日会错过至少六周的挽回窗口。", "建立基于活跃度、关键人变动和工单情绪的三级预警。", "整理过去半年流失客户的共同信号", "小周", "8 月 6 日", "预警分数应该由谁负责每月校准？", "验证"),
  rec("05", null, 5, "咖啡聊天：自由职业者的时间管理", "个人效率", "一次非正式交流中总结了自由职业者安排深度工作和客户沟通的方法。", "将客户沟通集中到固定时段能减少上下文切换。", "每周预留半天缓冲比排满日程更容易按时交付。", "试行上午深度工作、下午集中沟通的时间块安排。", "在日历中建立两周试行模板", "我", "明天", "突发客户需求如何不打断核心工作？", "新增"),
  rec("06", "demo-project-team", 6, "研发周会：交付节奏与阻塞项", "团队协作", "本周重点讨论了接口延期、测试环境不稳定和跨团队确认过慢。", "接口定义频繁变化是当前返工的主要来源。", "测试环境问题需要明确单一负责人而不是群里临时协调。", "每周三冻结当周接口，紧急变更必须附影响说明。", "建立接口变更记录并通知上下游", "陈工", "本周三", "测试环境 SLA 应该设为多长时间？", "修正"),
  rec("07", "demo-project-product", 18, "搜索体验可用性测试复盘", "搜索体验", "六位用户参与了搜索测试，暴露出筛选入口隐蔽和结果摘要不够明确的问题。", "用户更依赖结果摘要判断是否值得点开。", "筛选条件超过三个后，用户容易忘记当前范围。", "先改造摘要信息层级，并把已选筛选项固定展示在搜索框下方。", "完成新版结果卡片高保真原型", "小孟", "下周二", "是否需要给历史搜索提供一键复用？", "补充"),
  rec("08", "demo-project-growth", 25, "定价实验第一轮数据复盘", "商业化", "团队复盘了专业版两档价格的转化和退款情况。", "高价组短期转化下降，但高意向用户的试用完成率更高。", "退款原因主要是能力边界预期不一致，而非价格本身。", "继续保留两档价格，并重写试用页的能力边界说明。", "完成试用页价值说明文案 A/B 版本", "思思", "两周内", "高价组的三十日留存是否能抵消转化下降？", "修正"),
  rec("09", "demo-project-learning", 28, "《高效能人士》读书会", "自我管理", "读书会围绕主动选择、以终为始和重要不紧急事项展开讨论。", "真正有效的计划来自角色和长期目标，而不是任务堆积。", "每周回顾需要同时检查时间分配和情绪消耗。", "把周计划从任务清单改成按角色分配三个关键结果。", "设计一页式周复盘模板", "我", "周日晚上", "如何避免角色目标变成另一种形式的任务焦虑？", "补充"),
  rec("10", "demo-project-team", 35, "新成员入职流程复盘", "人才发展", "两位新成员反馈资料分散、首周目标不清晰，导师支持体验差异较大。", "入职第一周最需要的是清晰的成功标准和联系人地图。", "通用资料可以标准化，但岗位任务仍需直属负责人定制。", "采用统一清单加岗位 30 天任务包的双层结构。", "合并分散的入职文档并建立入口页", "HRBP", "本月底", "导师投入时间应该如何纳入团队容量？", "合并"),
  rec("11", "demo-project-life", 42, "年度体检结果与运动计划", "健康管理", "根据体检结果讨论了睡眠、力量训练和饮食调整的优先级。", "当前最需要改善的是睡眠规律，而不是增加高强度运动。", "每周两次力量训练比偶尔一次长时间训练更可持续。", "未来八周先固定睡眠窗口，并安排每周二、周六力量训练。", "购买弹力带并记录八周训练数据", "我", "本周", "如何判断睡眠改善是否真正带来白天精力提升？", "修正"),
  rec("12", "demo-project-product", 52, "离线转写技术方案评审", "技术架构", "工程团队比较了端侧模型、桌面本地服务和云端转写三种方案。", "端侧方案隐私最好，但模型包体和低端设备性能压力明显。", "桌面本地服务能兼顾隐私与模型迭代速度。", "桌面端优先采用本地服务架构，移动端先保留上传到桌面处理。", "完成 30 分钟音频的性能基准测试", "老王", "下个版本前", "本地服务崩溃后任务恢复机制如何设计？", "验证"),
  rec("13", null, 58, "行业播客摘记：垂直 AI 的机会", "行业观察", "播客讨论了垂直 AI 从工具走向工作流基础设施的机会。", "真正的壁垒来自持续积累的业务上下文，而不是单次生成效果。", "用户愿意为减少交接成本和责任不清付费。", "后续产品研究重点从功能清单转向上下文沉淀和闭环执行。", "整理三个垂直行业的工作流样本", "我", "本月", "上下文资产如何在隐私和可迁移性之间平衡？", "修正"),
  rec("14", "demo-project-growth", 65, "渠道合作伙伴季度复盘", "渠道增长", "团队复盘了五家渠道伙伴的线索质量、跟进速度和联合活动效果。", "高质量线索主要来自有明确场景的联合内容，而非泛流量活动。", "渠道跟进慢往往是因为缺少统一的客户画像字段。", "下一季度减少泛曝光活动，把预算集中到三场场景化联合研讨会。", "制定渠道线索交接字段和响应 SLA", "小郑", "下周", "如何激励渠道持续补全客户使用反馈？", "修正"),
  rec("15", "demo-project-growth", 180, "早期种子用户访谈总结", "用户需求", "早期访谈显示用户最看重快速回顾和可追溯引用，而不是复杂自动化。", "用户在会后十分钟内最需要找到决策和待办。", "没有原文引用的摘要很难被用于正式协作。", "MVP 首先交付转写、摘要、决策待办和原文跳转。", "整理 MVP 功能优先级并冻结范围", "产品组", "已完成", "个人记录与团队共享之间的边界如何设计？", "新增"),
  rec("16", "demo-project-team", 90, "半年度绩效校准会", "人才评估", "管理团队统一了绩效证据、潜力判断和跨团队贡献的校准口径。", "只看项目结果会低估承担基础设施工作的成员。", "评价需要区分结果、难度、协作影响和成长速度。", "绩效材料必须附具体行为和影响证据，禁止只写主观标签。", "更新绩效评审模板中的证据字段", "HRBP", "下周期前", "跨团队贡献由谁提供和确认最公平？", "修正"),
  rec("17", "demo-project-learning", 105, "设计系统案例研究", "设计系统", "研究比较了三家产品在设计令牌、组件治理和跨端协作上的实践。", "设计系统成功与否更多取决于治理流程，而不只是组件数量。", "变更日志和迁移指南是提升采用率的关键资产。", "设计系统建设先从高频基础组件和变更流程开始。", "盘点现有组件重复率和使用频次", "设计组", "下月", "如何量化设计系统对研发效率的实际贡献？", "新增"),
  rec("18", "demo-project-product", 120, "产品年度路线图工作坊", "产品战略", "工作坊明确了产品从录音工具向个人知识与行动系统演进的长期方向。", "用户价值不在保存更多录音，而在降低回顾和执行成本。", "跨记录关联必须可解释、可纠错，不能隐藏模型推断。", "年度路线图围绕捕获、理解、连接、行动四个阶段展开。", "把四阶段路线图拆成季度里程碑", "产品负责人", "已完成", "行动闭环应先集成日历还是任务系统？", "新增"),
  rec("19", "demo-project-life", 130, "家庭财务年度回顾", "财务规划", "家庭回顾了现金流、保险和长期储蓄目标。", "应急金已经达到六个月支出，可以降低现金闲置比例。", "大额旅行和教育支出需要单独账户，避免影响长期投资。", "保持六个月应急金，其余新增结余按月定投。", "建立旅行与教育两个专项预算账户", "我", "本季度", "明年是否需要调整保险保障额度？", "验证"),
  rec("20", null, 88, "散步灵感：知识库应该如何生长", "知识管理", "散步时记录了知识库从文件归档转向观点演化追踪的想法。", "知识价值来自观点之间的联系和变化，而不是文件数量。", "系统应允许用户确认或驳回模型推断，形成长期反馈。", "将认知演化和推断反馈作为知识产品的重要方向。", "画出观点变化链路的交互草图", "我", "有空时", "如何让用户理解推断置信度而不增加认知负担？", "新增"),
];

function rec(id, projectId, daysAgo, title, theme, summary, point1, point2, decision, action1, owner1, due1, question, changeType) {
  return { id: `demo-record-${id}`, projectId, daysAgo, title, theme, summary, point1, point2, decision, actions: [{ title: action1, owner: owner1, due: due1 }, { title: `跟踪「${title}」后续结果并在下次复盘更新`, owner: "相关负责人", due: "下次复盘" }], question, changeType };
}

function localIsoDaysAgo(days, hour = 10, minute = 0) {
  const date = new Date();
  date.setDate(date.getDate() - days);
  date.setHours(hour, minute, 0, 0);
  return date.toISOString();
}
function rangeBounds(key) {
  if (key === "all") return { start: null, end: null };
  const days = Number(key.slice(0, -1));
  const end = new Date();
  end.setHours(23, 59, 59, 999);
  const start = new Date(end);
  start.setDate(start.getDate() - days + 1);
  start.setHours(0, 0, 0, 0);
  return { start: start.toISOString(), end: end.toISOString() };
}
function q(value) {
  if (value === null || value === undefined) return "NULL";
  return `'${String(value).replaceAll("'", "''")}'`;
}
function j(value) { return q(JSON.stringify(value)); }
function source(record, segmentIndex = 0) {
  return { recordId: record.id, segmentId: `${record.id}-segment-${segmentIndex + 1}`, startMs: segmentIndex * 26000, endMs: segmentIndex * 26000 + 23000, quoteText: transcriptTexts(record)[segmentIndex] };
}
function transcriptTexts(record) {
  return [
    `今天围绕「${record.title}」同步背景。${record.summary}`,
    `当前最重要的观察是：${record.point1}`,
    `另一个需要保留的关键点是：${record.point2}`,
    `经过讨论，团队最终决定：${record.decision}`,
    `明确的下一步是：${record.actions[0].title}，由${record.actions[0].owner}负责，时间是${record.actions[0].due}。`,
    `会议最后留下一个待验证问题：${record.question}`,
  ];
}
function analysisFor(record) {
  return {
    summary: record.summary,
    key_points: [
      { text: record.point1, citation_segment_ids: [`${record.id}-segment-2`], quote_text: transcriptTexts(record)[1], start_ms: 26000, end_ms: 49000 },
      { text: record.point2, citation_segment_ids: [`${record.id}-segment-3`], quote_text: transcriptTexts(record)[2], start_ms: 52000, end_ms: 75000 },
      { text: `本次讨论围绕“${record.theme}”形成了可继续跟踪的共同认知。`, citation_segment_ids: [`${record.id}-segment-1`], quote_text: transcriptTexts(record)[0], start_ms: 0, end_ms: 23000 },
    ],
    decisions: [{ text: record.decision, citation_segment_ids: [`${record.id}-segment-4`], quote_text: transcriptTexts(record)[3], start_ms: 78000, end_ms: 101000 }],
    action_items: record.actions.map((item, index) => ({ text: item.title, citation_segment_ids: [`${record.id}-segment-${index === 0 ? 5 : 4}`], quote_text: transcriptTexts(record)[index === 0 ? 4 : 3], start_ms: index === 0 ? 104000 : 78000, end_ms: index === 0 ? 127000 : 101000 })),
    open_questions: [{ text: record.question, citation_segment_ids: [`${record.id}-segment-6`], quote_text: transcriptTexts(record)[5], start_ms: 130000, end_ms: 153000 }],
    custom_sections: [{ key: "signals", title: "风险与观察信号", format: "list", text: "", items: [{ text: `持续观察：${record.question}`, citation_segment_ids: [`${record.id}-segment-6`], quote_text: transcriptTexts(record)[5], start_ms: 130000, end_ms: 153000 }] }],
  };
}

function writeSilentWav(path, durationMs) {
  mkdirSync(dirname(path), { recursive: true });
  const sampleRate = 8000;
  const dataSize = Math.floor(sampleRate * 2 * durationMs / 1000);
  const header = Buffer.alloc(44);
  header.write("RIFF", 0); header.writeUInt32LE(36 + dataSize, 4); header.write("WAVEfmt ", 8);
  header.writeUInt32LE(16, 16); header.writeUInt16LE(1, 20); header.writeUInt16LE(1, 22);
  header.writeUInt32LE(sampleRate, 24); header.writeUInt32LE(sampleRate * 2, 28);
  header.writeUInt16LE(2, 32); header.writeUInt16LE(16, 34); header.write("data", 36); header.writeUInt32LE(dataSize, 40);
  const fd = openSync(path, "w");
  writeSync(fd, header); ftruncateSync(fd, 44 + dataSize); closeSync(fd);
}

const sql = ["PRAGMA trusted_schema = ON;", "PRAGMA foreign_keys = ON;", "BEGIN IMMEDIATE;"];
sql.push("DELETE FROM memory_feedback WHERE id LIKE 'demo-%' OR snapshot_id IN (SELECT id FROM memory_snapshots WHERE id LIKE 'demo-%');");
sql.push("DELETE FROM memory_snapshots WHERE id LIKE 'demo-%';");
sql.push("DELETE FROM record_search WHERE record_id LIKE 'demo-%' OR source_id LIKE 'demo-%';");
sql.push("DELETE FROM citations WHERE id LIKE 'demo-%' OR analysis_id LIKE 'demo-%';");
sql.push("DELETE FROM action_items WHERE id LIKE 'demo-%' OR record_id LIKE 'demo-%';");
sql.push("DELETE FROM analyses WHERE id LIKE 'demo-%' OR record_id LIKE 'demo-%';");
sql.push("DELETE FROM transcript_segments WHERE id LIKE 'demo-%' OR record_id LIKE 'demo-%';");
sql.push("DELETE FROM transcript_versions WHERE id LIKE 'demo-%' OR record_id LIKE 'demo-%';");
sql.push("DELETE FROM processing_jobs WHERE id LIKE 'demo-%' OR record_id LIKE 'demo-%';");
sql.push("DELETE FROM records WHERE id LIKE 'demo-%';");
sql.push("DELETE FROM projects WHERE id LIKE 'demo-%';");

const now = new Date().toISOString();
for (const project of projects) {
  sql.push(`INSERT INTO projects (id,name,description,status,created_at,updated_at) VALUES (${q(project.id)},${q(project.name)},${q(project.description)},'active',${q(localIsoDaysAgo(200))},${q(now)});`);
}

for (const record of records) {
  const importedAt = localIsoDaysAgo(record.daysAgo, 9 + Number(record.id.slice(-2)) % 8, 10);
  const audioRel = `audio/demo/${record.id}.wav`;
  const durationMs = 156000;
  writeSilentWav(join(libraryRoot, audioRel), durationMs);
  sql.push(`INSERT INTO records (id,title,project_id,source_type,audio_path,audio_hash,audio_duration_ms,imported_at,processing_status,created_at,updated_at,analysis_template_id) VALUES (${q(record.id)},${q(record.title)},${q(record.projectId)},'import',${q(audioRel)},${q(`demo-hash-${record.id}`)},${durationMs},${q(importedAt)},'completed',${q(importedAt)},${q(importedAt)},'builtin-standard');`);
  for (const jobType of ["transcribe", "analyze"]) {
    sql.push(`INSERT INTO processing_jobs (id,record_id,job_type,status,attempt_count,last_error,created_at,updated_at,stage,progress_current,progress_total) VALUES (${q(`demo-job-${record.id}-${jobType}`)},${q(record.id)},${q(jobType)},'completed',1,NULL,${q(importedAt)},${q(importedAt)},'completed',100,100);`);
  }
  const versionId = `demo-transcript-${record.id}`;
  sql.push(`INSERT INTO transcript_versions (id,record_id,provider,model,status,created_at,language,pipeline_version,preprocessing_json) VALUES (${q(versionId)},${q(record.id)},'demo','mock-zh-v1','completed',${q(importedAt)},'zh','demo-v1','{}');`);
  const texts = transcriptTexts(record);
  texts.forEach((text, index) => {
    const segmentId = `${record.id}-segment-${index + 1}`;
    const startMs = index * 26000;
    sql.push(`INSERT INTO transcript_segments (id,transcript_version_id,record_id,sequence,speaker_label,start_ms,end_ms,original_text,edited_text,created_at,updated_at,normalized_text,normalization_version) VALUES (${q(segmentId)},${q(versionId)},${q(record.id)},${index},${q(index % 2 ? "参与者" : "主持人")},${startMs},${startMs + 23000},${q(text)},NULL,${q(importedAt)},${q(importedAt)},${q(text)},'demo-v1');`);
  });
  const analysisId = `demo-analysis-${record.id}`;
  const analysis = analysisFor(record);
  sql.push(`INSERT INTO analyses (id,record_id,source_transcript_version_id,status,content_json,provider,model,template_version,created_at,template_id,template_snapshot_json) VALUES (${q(analysisId)},${q(record.id)},${q(versionId)},'completed',${j(analysis)},'demo','mock-analysis-v1','demo-v1',${q(importedAt)},'builtin-standard','');`);
  sql.push(`INSERT INTO record_search (record_id,project_id,source_id,title,body) VALUES (${q(record.id)},${q(record.projectId)},${q(analysisId)},${q(record.title)},${q([record.summary, record.point1, record.point2, record.decision, ...record.actions.map(a => a.title), record.question].join(" "))});`);
  const cited = [...analysis.key_points.map((item, i) => [`key_points[${i}]`, item]), ["decisions[0]", analysis.decisions[0]], ...analysis.action_items.map((item, i) => [`action_items[${i}]`, item]), ["open_questions[0]", analysis.open_questions[0]], ["custom_sections[0].items[0]", analysis.custom_sections[0].items[0]]];
  cited.forEach(([itemPath, item], index) => sql.push(`INSERT INTO citations (id,analysis_id,item_path,transcript_segment_id,quote_text,verified) VALUES (${q(`demo-citation-${record.id}-${index + 1}`)},${q(analysisId)},${q(itemPath)},${q(item.citation_segment_ids[0])},${q(item.quote_text)},1);`));
  record.actions.forEach((item, index) => sql.push(`INSERT INTO action_items (id,record_id,project_id,title,owner_text,due_text,status,source_segment_id,analysis_id) VALUES (${q(`demo-action-${record.id}-${index + 1}`)},${q(record.id)},${q(record.projectId)},${q(item.title)},${q(item.owner)},${q(item.due)},${q(index === 1 && record.daysAgo > 60 ? "done" : "open")},${q(`${record.id}-segment-${index === 0 ? 5 : 4}`)},${q(analysisId)});`));
}

const scopes = [{ kind: "all", key: "all", projectId: null }, ...projects.map(p => ({ kind: "project", key: `project:${p.id}`, projectId: p.id })), { kind: "unfiled", key: "unfiled", projectId: null }];
for (const scope of scopes) {
  for (const rangeKey of ["7d", "30d", "90d", "all"]) {
    const bounds = rangeBounds(rangeKey);
    const eligible = records.filter(record => (scope.kind === "all" || (scope.kind === "project" ? record.projectId === scope.projectId : record.projectId === null)) && (rangeKey === "all" || new Date(localIsoDaysAgo(record.daysAgo)) >= new Date(bounds.start)));
    for (const viewKind of ["timeline", "map", "evolution"]) {
      const snapshotId = `demo-snapshot-${viewKind}-${scope.key.replaceAll(":", "-")}-${rangeKey}`;
      const result = snapshotResult(viewKind, eligible, scope, rangeKey);
      const versionExpr = `(SELECT COALESCE(MAX(version),0)+1 FROM memory_snapshots WHERE view_kind=${q(viewKind)} AND scope_key=${q(scope.key)} AND range_start IS ${q(bounds.start)} AND range_end IS ${q(bounds.end)})`;
      sql.push(`INSERT INTO memory_snapshots (id,view_kind,scope_kind,scope_key,range_start,range_end,status,provider,model,source_record_ids_json,request_hash,result_json,quality_warning,error_message,is_stale,version,created_at,updated_at) VALUES (${q(snapshotId)},${q(viewKind)},${q(scope.kind)},${q(scope.key)},${q(bounds.start)},${q(bounds.end)},'completed','demo','mock-memory-v1',${j(eligible.map(r => r.id))},${q(`demo-${viewKind}-${scope.key}-${rangeKey}`)},${j(result)},NULL,NULL,0,${versionExpr},${q(now)},${q(now)});`);
      if (viewKind === "evolution" && rangeKey === "90d" && result.evolutionItems.length) {
        const first = result.evolutionItems[0];
        sql.push(`INSERT INTO memory_feedback (id,snapshot_id,item_id,decision,note,created_at,updated_at) VALUES (${q(`demo-feedback-${scope.key.replaceAll(":", "-")}-1`)},${q(snapshotId)},${q(first.id)},'confirmed','这条变化符合当时的讨论背景。',${q(now)},${q(now)});`);
      }
    }
  }
}

sql.push("COMMIT;");
runSql(sql.join("\n"));

const counts = runSql(`SELECT 'projects',COUNT(*) FROM projects WHERE id LIKE 'demo-%' UNION ALL SELECT 'records',COUNT(*) FROM records WHERE id LIKE 'demo-%' UNION ALL SELECT 'segments',COUNT(*) FROM transcript_segments WHERE id LIKE 'demo-%' UNION ALL SELECT 'analyses',COUNT(*) FROM analyses WHERE id LIKE 'demo-%' UNION ALL SELECT 'actions',COUNT(*) FROM action_items WHERE id LIKE 'demo-%' UNION ALL SELECT 'snapshots',COUNT(*) FROM memory_snapshots WHERE id LIKE 'demo-%';`);
console.log(`已写入演示数据：\n${counts}\n数据库：${dbPath}`);

function snapshotResult(viewKind, eligible, scope, rangeKey) {
  const empty = { timelineItems: [], nodes: [], edges: [], evolutionItems: [], dormantQuestions: [], stalledProjects: [] };
  if (!eligible.length) return empty;
  if (viewKind === "timeline") {
    const latestByProject = new Map();
    for (const record of eligible.slice().sort((a, b) => a.daysAgo - b.daysAgo)) {
      const key = record.projectId || "unfiled";
      if (!latestByProject.has(key)) latestByProject.set(key, record);
    }
    empty.timelineItems = [...latestByProject.values()].map((record, index) => ({ id: `demo-timeline-inference-${scope.key.replaceAll(":", "-")}-${rangeKey}-${index}`, occurredAt: localIsoDaysAgo(record.daysAgo, 18), itemType: "event", title: `${record.theme}形成阶段性进展`, summary: `多次讨论逐步收敛到：${record.decision}`, projectId: record.projectId, projectName: projects.find(p => p.id === record.projectId)?.name || null, inferred: true, confidence: 0.82 - index * 0.03, sources: [source(record, 3)] }));
    return empty;
  }
  if (viewKind === "map") {
    const sample = eligible.slice().sort((a, b) => a.daysAgo - b.daysAgo).slice(0, 8);
    const projectIds = [...new Set(sample.map(r => r.projectId || "unfiled"))];
    for (const [index, projectId] of projectIds.entries()) {
      const first = sample.find(r => (r.projectId || "unfiled") === projectId);
      empty.nodes.push({ id: `demo-map-project-${scope.key.replaceAll(":", "-")}-${rangeKey}-${index}`, nodeType: "project", label: projectId === "unfiled" ? "未归档灵感" : projects.find(p => p.id === projectId)?.name, summary: `该范围内共有 ${sample.filter(r => (r.projectId || "unfiled") === projectId).length} 条相关记录。`, inferred: true, confidence: 0.96, sources: [source(first)] });
    }
    sample.forEach((record, index) => {
      const rootIndex = projectIds.indexOf(record.projectId || "unfiled");
      const rootId = `demo-map-project-${scope.key.replaceAll(":", "-")}-${rangeKey}-${rootIndex}`;
      const themeId = `demo-map-theme-${scope.key.replaceAll(":", "-")}-${rangeKey}-${index}`;
      const decisionId = `demo-map-decision-${scope.key.replaceAll(":", "-")}-${rangeKey}-${index}`;
      empty.nodes.push({ id: themeId, nodeType: "theme", label: record.theme, summary: record.summary, inferred: true, confidence: 0.88, sources: [source(record)] });
      empty.nodes.push({ id: decisionId, nodeType: "decision", label: record.decision, summary: `由「${record.title}」形成的阶段性决策。`, inferred: true, confidence: 0.91, sources: [source(record, 3)] });
      empty.edges.push({ id: `demo-map-edge-theme-${scope.key.replaceAll(":", "-")}-${rangeKey}-${index}`, sourceId: themeId, targetId: rootId, relation: "属于", inferred: true, confidence: 0.93, sources: [source(record)] });
      empty.edges.push({ id: `demo-map-edge-decision-${scope.key.replaceAll(":", "-")}-${rangeKey}-${index}`, sourceId: decisionId, targetId: themeId, relation: "推动", inferred: true, confidence: 0.86, sources: [source(record, 3)] });
    });
    return empty;
  }
  const sample = eligible.slice().sort((a, b) => b.daysAgo - a.daysAgo).slice(-6);
  empty.evolutionItems = sample.map((record, index) => ({ id: `demo-evolution-${scope.key.replaceAll(":", "-")}-${rangeKey}-${index}`, topic: record.theme, changeType: record.changeType, beforeText: index === 0 ? "此前只有零散观察，尚未形成统一做法。" : sample[index - 1].point1, afterText: record.decision, reason: `${record.title}补充了新的事实与讨论证据。`, occurredAt: localIsoDaysAgo(record.daysAgo, 17), inferred: true, confidence: 0.91 - index * 0.035, sources: [source(record, 3)] }));
  empty.dormantQuestions = eligible.slice().sort((a, b) => b.daysAgo - a.daysAgo).slice(0, 3).map(record => record.question);
  empty.stalledProjects = eligible.some(record => record.daysAgo > 60) ? [scope.kind === "project" ? `${projects.find(p => p.id === scope.projectId)?.name}仍有历史问题等待形成里程碑` : "部分长期议题有持续讨论，但缺少明确验收里程碑"] : [];
  return empty;
}
