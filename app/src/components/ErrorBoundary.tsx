import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
}

/**
 * 全应用渲染错误兜底：任何组件抛错时展示可恢复界面，
 * 避免整个窗口白屏且没有任何提示。资料库数据在后端 SQLite 中，不受影响。
 */
export default class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("回声记忆界面错误：", error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <main className="app-shell" role="alert">
        <div className="error-boundary">
          <h1>界面出现了一点问题</h1>
          <p>
            你的录音、逐字稿和分析都安全保存在本机资料库中，没有丢失。
            重新加载界面即可继续使用；如果反复出现此提示，请通过设置中的渠道反馈。
          </p>
          <p className="error-boundary-detail">{String(this.state.error?.message ?? this.state.error)}</p>
          <button type="button" className="primary-button" onClick={() => window.location.reload()}>
            重新加载界面
          </button>
        </div>
      </main>
    );
  }
}
