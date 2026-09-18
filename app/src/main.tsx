import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import ErrorBoundary from "./components/ErrorBoundary";
import { initTheme } from "./lib/theme";
import "./styles.css";

// index.html 的内联脚本已经在 CSS 前挂过一次 data-theme，这里再跑一遍
// 是为了把「存储值」带回 React 世界（主题设置卡片直接读它）。
initTheme();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>
);
