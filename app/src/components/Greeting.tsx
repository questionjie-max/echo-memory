import { useState } from "react";
import type { AppInfo } from "../shared/types";
import { getAppInfo, generateId, greet } from "../lib/tauri";

/**
 * 演示组件：通过类型化封装调用三个后端命令，
 * 展示「React → Tauri 命令 → Rust」的通信闭环与共享类型的端到端正确性。
 */
export default function Greeting() {
  const [name, setName] = useState("");
  const [greetMsg, setGreetMsg] = useState("");
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  const [newId, setNewId] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function runGreet() {
    setBusy(true);
    setError("");
    try {
      setGreetMsg(await greet(name || "世界"));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function loadAppInfo() {
    setBusy(true);
    setError("");
    try {
      setAppInfo(await getAppInfo());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function makeId() {
    setBusy(true);
    setError("");
    try {
      setNewId(await generateId());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section style={{ display: "grid", gap: 16, marginTop: 24 }}>
      <h2 style={{ fontSize: 18 }}>前端 → 后端命令调用示例</h2>

      <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
        <input
          value={name}
          placeholder="输入名字"
          onChange={(e) => setName(e.target.value)}
          style={{ padding: 8, borderRadius: 6, border: "1px solid #cbd2d9" }}
        />
        <button onClick={runGreet} disabled={busy}>
          问候（greet）
        </button>
      </div>
      {greetMsg && (
        <p>
          <strong>返回：</strong>
          {greetMsg}
        </p>
      )}

      <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
        <button onClick={loadAppInfo} disabled={busy}>
          获取应用信息（app_info）
        </button>
        <button onClick={makeId} disabled={busy}>
          生成 UUID（generate_id）
        </button>
      </div>

      {appInfo && (
        <pre
          style={{
            background: "#f5f7fa",
            padding: 12,
            borderRadius: 8,
            overflowX: "auto",
          }}
        >
          {JSON.stringify(appInfo, null, 2)}
        </pre>
      )}
      {newId && (
        <p>
          <strong>新 UUID：</strong>
          <code>{newId}</code>
        </p>
      )}

      {error && (
        <p style={{ color: "#c0392b" }}>错误：{error}</p>
      )}
    </section>
  );
}
