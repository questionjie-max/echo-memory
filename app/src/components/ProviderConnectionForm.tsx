import { useEffect, useMemo, useRef, useState } from "react";
import type {
  DiagnosisReport,
  ExternalAiSettings,
  ProviderCapability,
  ProviderCatalog,
  ProviderDefinition,
  ProviderHealthSnapshot,
  RemoteModelList,
  SaveAndVerifyOutcome,
} from "../shared/types";
import { describeDiagnosis } from "../lib/diagnosis-copy";
import { deriveVerifyState, failedDescription, verifiedDescription } from "../lib/verify-state";
import { Button, Collapsible, ErrorNote, Field, VerifyStatusPill, type ErrorNoteModel } from "./SettingsKit";

export type ProviderTarget = "analysis" | "transcription";

export interface ProviderCommit {
  providerId: string;
  baseUrl: string;
  model: string;
  credentials: Record<string, string> | null;
  consentAt: string;
}

/** 草稿诊断入参：凭据可为空表（后端回退已保存凭据）。 */
export interface ProviderDiagnoseDraft {
  target: ProviderTarget;
  providerId: string;
  baseUrl: string;
  model: string;
  credentials: Record<string, string>;
}

/** 保存并验证入参：携带一次性验证票据。 */
export interface ProviderSaveAndVerifyDraft extends ProviderDiagnoseDraft {
  consentAt: string;
  enabled: boolean;
  verificationToken: string;
}

export interface ProviderConnectionFormProps {
  target: ProviderTarget;
  catalog: ProviderCatalog;
  settings: ExternalAiSettings;
  busy: boolean;
  actionLabel: string;
  health: ProviderHealthSnapshot | null;
  credentialRevision: number;
  catalogVersion: number;
  /** 仅保存（跳过验证）：提交后状态为「已配置，未验证」。 */
  onCommit: (draft: ProviderCommit) => Promise<void>;
  /** 草稿诊断：不落盘的真实小请求；通过时返回一次性验证票据。 */
  onDiagnose: (draft: ProviderDiagnoseDraft) => Promise<DiagnosisReport>;
  /** 保存并验证：先核销票据再提交；验证失败原配置不变。 */
  onSaveAndVerify: (draft: ProviderSaveAndVerifyDraft) => Promise<SaveAndVerifyOutcome>;
  /** 验证当前已保存配置（对已存凭据的真实小请求）。 */
  onVerifySaved: () => Promise<DiagnosisReport>;
  onClearCredentials: () => Promise<void>;
  onReuseCredentials: (draft: Omit<ProviderCommit, "credentials" | "consentAt">) => Promise<void>;
  onRevealCredentials?: (providerId: string) => Promise<Record<string, string>>;
  onListModels?: (input: {
    providerId: string;
    baseUrl: string;
    apiKey: string | null;
  }) => Promise<RemoteModelList>;
}

export default function ProviderConnectionForm({
  target,
  catalog,
  settings,
  busy,
  actionLabel,
  health,
  credentialRevision,
  catalogVersion,
  onCommit,
  onDiagnose,
  onSaveAndVerify,
  onVerifySaved,
  onClearCredentials,
  onReuseCredentials,
  onRevealCredentials,
  onListModels,
}: ProviderConnectionFormProps) {
  const providers = useMemo(
    () => catalog.providers.filter((provider) => provider.capabilities.includes(target)),
    [catalog, target],
  );
  const storedProvider = target === "analysis"
    ? settings.analysisProvider
    : settings.transcriptionProvider;
  const initialProvider = eligibleProvider(providers, storedProvider) ?? providers[0];
  const storedMatches = initialProvider?.id === storedProvider;
  const [providerId, setProviderId] = useState(initialProvider?.id ?? "");
  const [baseUrl, setBaseUrl] = useState(
    storedMatches
      ? storedBaseUrl(settings, target)
      : capabilityFor(initialProvider, target)?.baseUrl ?? "",
  );
  const [model, setModel] = useState(
    storedMatches && storedModel(settings, target).trim()
      ? storedModel(settings, target)
      : capabilityFor(initialProvider, target)?.recommendedModel ?? "",
  );
  const [credentials, setCredentials] = useState<Record<string, string>>({});
  const [revealed, setRevealed] = useState<Record<string, boolean>>({});
  const [consent, setConsent] = useState(Boolean(storedConsent(settings, target)));
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [errorNote, setErrorNote] = useState<ErrorNoteModel | null>(null);
  const [working, setWorking] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [editingCredentials, setEditingCredentials] = useState(false);
  const [showingSaved, setShowingSaved] = useState(false);
  const [remoteModels, setRemoteModels] = useState<RemoteModelList | null>(null);
  const [loadingModels, setLoadingModels] = useState(false);
  /** 请求序号守卫：旧响应一律丢弃，切换服务商即作废旧序号。 */
  const requestSeq = useRef(0);

  const provider = providers.find((item) => item.id === providerId) ?? null;
  const capability = capabilityFor(provider, target);
  const serverMatchesDraft = providerId === storedProvider;
  const savedCredentials = serverMatchesDraft && storedHasCredentials(settings, target);
  const fields = provider?.credentialFields ?? [];
  const enteredFields = fields.filter((field) => credentials[field.key]?.trim());
  const allFieldsEntered =
    fields.length > 0 &&
    fields.every((field) => Boolean(credentials[field.key]?.trim()));
  const credentialsReady =
    allFieldsEntered ||
    (!editingCredentials && enteredFields.length === 0 && savedCredentials);
  const customUrlMissing = providerId === "custom" && !baseUrl.trim();
  const canSubmit =
    Boolean(provider && capability && model.trim() && !customUrlMissing) &&
    credentialsReady &&
    consent &&
    !busy &&
    !working &&
    !verifying &&
    !loadingModels;
  const otherProvider =
    target === "analysis" ? settings.transcriptionProvider : settings.analysisProvider;
  const otherHasCredentials =
    target === "analysis" ? settings.transcriptionHasApiKey : settings.hasApiKey;
  const canReuse = otherHasCredentials && otherProvider === providerId && Boolean(providerId);
  const modelNeedsInput =
    providerId === "custom" || providerId === "openai";
  const chatModels = remoteModels?.chatCandidates ?? [];
  const otherRemoteModels = remoteModels?.otherModels ?? [];
  const builtInModels = capability?.models ?? [];
  const listedModel =
    builtInModels.includes(model) || chatModels.includes(model) || otherRemoteModels.includes(model);
  const manualModelField = Boolean(capability?.models.length) && !modelNeedsInput;
  const backendLocked = busy || working || verifying || loadingModels;
  const enteredApiKey = credentials.apiKey?.trim() ?? "";
  const canFetchModels =
    Boolean(onListModels && capability?.supportsModelList) &&
    !backendLocked &&
    !customUrlMissing &&
    (savedCredentials || Boolean(enteredApiKey));
  const draftMatchesSaved =
    serverMatchesDraft &&
    baseUrl.trim() === storedBaseUrl(settings, target).trim() &&
    model.trim() === storedModel(settings, target).trim();
  const hasUnsavedCredentialInput = enteredFields.length > 0 && !showingSaved;
  const savedConfigVerified =
    Boolean(onVerifySaved) &&
    savedCredentials &&
    Boolean(storedConsent(settings, target)) &&
    draftMatchesSaved &&
    !hasUnsavedCredentialInput;
  // 草稿偏离已保存配置：provider/地址/模型被编辑，或凭据字段被动过。
  const draftDeviation = !draftMatchesSaved || hasUnsavedCredentialInput || editingCredentials;
  const verifyState = deriveVerifyState({
    hasCredentials: storedHasCredentials(settings, target),
    providerId: storedProvider,
    baseUrl: storedBaseUrl(settings, target),
    model: storedModel(settings, target),
    health,
    credentialRevision,
    catalogVersion,
    draftDeviation,
    busy: busy || working || verifying,
  });

  useEffect(() => {
    const nextProvider = eligibleProvider(providers, storedProvider) ?? providers[0];
    const nextCapability = capabilityFor(nextProvider, target);
    setProviderId(nextProvider?.id ?? "");
    setBaseUrl(
      nextProvider?.id === storedProvider
        ? storedBaseUrl(settings, target)
        : nextCapability?.baseUrl ?? "",
    );
    setModel(
      nextProvider?.id === storedProvider && storedModel(settings, target).trim()
        ? storedModel(settings, target)
        : nextCapability?.recommendedModel ?? "",
    );
    setCredentials({});
    setRevealed({});
    setConsent(Boolean(storedConsent(settings, target)));
    setErrorNote(null);
    setEditingCredentials(false);
    setShowingSaved(false);
    setRemoteModels(null);
    setLoadingModels(false);
    setVerifying(false);
    requestSeq.current += 1;
    // Only server-side provider, endpoint, model and consent changes should reset
    // the draft. Local unsaved edits keep their state while unrelated props update.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    target,
    storedProvider,
    storedBaseUrl(settings, target),
    storedModel(settings, target),
    storedConsent(settings, target),
    storedHasCredentials(settings, target),
  ]);

  function chooseProvider(nextId: string) {
    const next = providers.find((item) => item.id === nextId) ?? null;
    const nextCapability = capabilityFor(next, target);
    requestSeq.current += 1;
    setProviderId(nextId);
    setBaseUrl(nextCapability?.baseUrl ?? "");
    setModel(nextCapability?.recommendedModel ?? "");
    setCredentials({});
    setRevealed({});
    setConsent(false);
    setAdvancedOpen(false);
    setErrorNote(null);
    setEditingCredentials(false);
    setShowingSaved(false);
    setRemoteModels(null);
    setLoadingModels(false);
  }

  function changeModel(nextModel: string) {
    setModel(nextModel);
    setConsent(false);
  }

  function buildDraft(): ProviderCommit | null {
    if (!provider) return null;
    const newCredentials = enteredFields.length > 0
      ? Object.fromEntries(
          fields.map((field) => [field.key, credentials[field.key]?.trim() ?? ""]),
        )
      : null;
    return {
      providerId: provider.id,
      baseUrl: baseUrl.trim(),
      model: model.trim(),
      credentials: newCredentials,
      consentAt: new Date().toISOString(),
    };
  }

  /** 诊断/提交用的凭据集合：新输入优先；没有输入则空表（后端回退已保存）。 */
  function draftCredentials(): Record<string, string> {
    if (enteredFields.length === 0) return {};
    return Object.fromEntries(
      fields.map((field) => [field.key, credentials[field.key]?.trim() ?? ""]),
    );
  }

  async function run(action: () => Promise<void>, fallback: string) {
    setWorking(true);
    setErrorNote(null);
    try {
      await action();
    } catch (reason) {
      setErrorNote({
        what: fallback,
        cause: String(reason),
        action: "检查后重试；若已部分保存，请关闭设置页重开，以页面显示为准",
      });
    } finally {
      setWorking(false);
    }
  }

  async function submit() {
    const draft = buildDraft();
    if (!draft || !canSubmit) return;
    await run(async () => {
      await onCommit(draft);
      setCredentials({});
      setRevealed({});
      setConsent(true);
      setEditingCredentials(false);
      setShowingSaved(false);
    }, "保存第三方服务配置失败");
  }

  /** 主流程：先诊断草稿（不落盘），通过后凭票据提交。失败零落盘。 */
  async function submitAndVerify() {
    if (!provider || !canSubmit) return;
    const seq = ++requestSeq.current;
    setVerifying(true);
    setErrorNote(null);
    try {
      const report = await onDiagnose({
        target,
        providerId: provider.id,
        baseUrl: baseUrl.trim(),
        model: model.trim(),
        credentials: draftCredentials(),
      });
      if (seq !== requestSeq.current) return;
      if (report.status !== "passed" || !report.verificationToken) {
        setErrorNote(describeDiagnosis(report));
        return;
      }
      await onSaveAndVerify({
        target,
        providerId: provider.id,
        baseUrl: baseUrl.trim(),
        model: model.trim(),
        credentials: draftCredentials(),
        consentAt: new Date().toISOString(),
        enabled: target === "analysis" ? true : settings.enabled,
        verificationToken: report.verificationToken,
      });
      if (seq !== requestSeq.current) return;
      setCredentials({});
      setRevealed({});
      setConsent(true);
      setEditingCredentials(false);
      setShowingSaved(false);
    } catch (reason) {
      if (seq === requestSeq.current) {
        setErrorNote({
          what: "保存并验证未完成",
          cause: String(reason),
          action: "按提示处理后重试；已保存的旧配置不受影响",
        });
      }
    } finally {
      if (seq === requestSeq.current) setVerifying(false);
    }
  }

  /** 对已保存配置做真实验证（结果写入健康快照）。 */
  async function verifySaved() {
    if (!savedConfigVerified) return;
    const seq = ++requestSeq.current;
    setVerifying(true);
    setErrorNote(null);
    try {
      const report = await onVerifySaved();
      if (seq !== requestSeq.current) return;
      if (report.status !== "passed") {
        setErrorNote(describeDiagnosis(report));
      }
    } catch (reason) {
      if (seq === requestSeq.current) {
        setErrorNote({
          what: "验证未完成",
          cause: String(reason),
          action: "检查后重试",
        });
      }
    } finally {
      if (seq === requestSeq.current) setVerifying(false);
    }
  }

  async function reuse() {
    if (!provider || !canReuse) return;
    await run(async () => {
      await onReuseCredentials({
        providerId: provider.id,
        baseUrl: baseUrl.trim(),
        model: model.trim(),
      });
      setCredentials({});
      setRevealed({});
      setConsent(false);
    }, "复用凭据失败");
  }

  async function clearCredentials() {
    await run(async () => {
      await onClearCredentials();
      setCredentials({});
      setRevealed({});
      setEditingCredentials(false);
      setShowingSaved(false);
    }, "清除凭据失败");
  }

  async function revealSavedCredentials() {
    if (!provider || !onRevealCredentials) return;
    await run(async () => {
      const values = await onRevealCredentials(provider.id);
      setCredentials(
        Object.fromEntries(
          fields.map((field) => [field.key, values[field.key] ?? ""]),
        ),
      );
      setRevealed(
        Object.fromEntries(fields.map((field) => [field.key, true])),
      );
      setShowingSaved(true);
    }, "无法显示已保存凭据");
  }

  function hideSavedCredentials() {
    setCredentials({});
    setRevealed({});
    setShowingSaved(false);
  }

  function beginCredentialEdit() {
    setCredentials({});
    setRevealed({});
    setShowingSaved(false);
    setEditingCredentials(true);
    setErrorNote(null);
  }

  function cancelCredentialEdit() {
    setCredentials({});
    setRevealed({});
    setEditingCredentials(false);
    setErrorNote(null);
  }

  async function fetchModels() {
    if (!provider || !onListModels || !canFetchModels) return;
    const seq = ++requestSeq.current;
    setLoadingModels(true);
    setErrorNote(null);
    try {
      const models = await onListModels({
        providerId: provider.id,
        baseUrl: baseUrl.trim(),
        apiKey: enteredApiKey || null,
      });
      if (seq !== requestSeq.current) return;
      if (models.chatCandidates.length === 0 && models.otherModels.length === 0) {
        setErrorNote({
          what: "服务商没有返回可用模型",
          cause: "模型目录为空或该账号没有可用模型",
          action: "改用内置推荐模型或手动填写模型 ID",
        });
        setRemoteModels(null);
        return;
      }
      setRemoteModels(models);
    } catch (reason) {
      if (seq === requestSeq.current) {
        setErrorNote({
          what: "拉取模型失败",
          cause: String(reason),
          action: "检查 Key 与网络后重试；也可以直接使用内置推荐模型",
        });
      }
    } finally {
      if (seq === requestSeq.current) setLoadingModels(false);
    }
  }

  if (!providers.length) {
    return <p className="em-note err">服务商目录尚未提供该能力。</p>;
  }

  const verifyDescription =
    verifyState === "verified"
      ? verifiedDescription(health)
      : verifyState === "failed"
        ? failedDescription(health)
        : null;
  // 当前模型不在实时列表时的对账警告：只对声明了模型列表接口的服务商生效，
  // 且绝不自动替换模型。
  const currentModelMissing =
    Boolean(remoteModels) &&
    capability?.supportsModelList &&
    model.trim() &&
    !listedModel;

  return (
    <>
      <Field
        label="服务商"
        hint="接口地址、协议和推荐模型会自动配置。"
      >
        <select
          className="em-select"
          aria-label={`${target === "analysis" ? "分析" : "转写"}服务商`}
          value={providerId}
          disabled={backendLocked}
          onChange={(event) => chooseProvider(event.target.value)}
        >
          {!provider && <option value="">请选择服务商</option>}
          {providers.map((item) => (
            <option value={item.id} key={item.id}>
              {item.label}
            </option>
          ))}
        </select>
      </Field>

      {provider && (
        <>
          <VerifyStatusPill state={verifyState} description={verifyDescription} />
          <p className="em-hint">{provider.description}</p>
          {provider.keyUrl && (
            <p className="em-hint">
              还没有 Key？
              <a href={provider.keyUrl} target="_blank" rel="noreferrer">
                前往 {provider.label} 获取
              </a>
            </p>
          )}

          {providerId === "custom" && (
            <Field
              label={target === "analysis" ? "接口地址" : "转写接口地址"}
              hint="填写 OpenAI 兼容的 /v1 地址。只有本机地址允许使用 http。"
            >
              <input
                className="em-input"
                value={baseUrl}
                placeholder="https://api.example.com/v1"
                disabled={backendLocked}
                onChange={(event) => {
                  setBaseUrl(event.target.value);
                  setConsent(false);
                }}
              />
            </Field>
          )}

          {modelNeedsInput ? (
            <Field
              label="模型 ID"
              hint={
                capability?.recommendedModel
                  ? `推荐 ${capability.recommendedModel}，也可填写该服务商支持的其它模型。`
                  : "填写服务商文档中的模型 ID。"
              }
            >
              <input
                className="em-input"
                value={model}
                disabled={backendLocked}
                list={`${target}-model-options`}
                onChange={(event) => changeModel(event.target.value)}
              />
              <datalist id={`${target}-model-options`}>
                {chatModels.length > 0 && (
                  <optgroup label="推荐 / 远端可用">
                    {[...new Set([...builtInModels, ...chatModels])].map((name) => (
                      <option value={name} key={name} />
                    ))}
                  </optgroup>
                )}
                {otherRemoteModels.length > 0 && (
                  <optgroup label="其他模型（不推荐用于此能力）">
                    {otherRemoteModels.map((name) => (
                      <option value={name} key={name} />
                    ))}
                  </optgroup>
                )}
              </datalist>
            </Field>
          ) : (
            <Field label="模型" hint={`默认使用推荐模型 ${capability?.recommendedModel}。`}>
              <div className="em-field-row">
                <select
                  className="em-select"
                  aria-label={`${target === "analysis" ? "分析" : "转写"}模型`}
                  data-testid={`${target}-model-select`}
                  value={model}
                  disabled={backendLocked}
                  onChange={(event) => changeModel(event.target.value)}
                >
                  {remoteModels ? (
                    <>
                      <optgroup label="推荐 / 远端可用">
                        {[...new Set([...builtInModels, ...chatModels])].map((name) => (
                          <option value={name} key={name}>
                            {name}
                          </option>
                        ))}
                      </optgroup>
                      {otherRemoteModels.length > 0 && (
                        <optgroup label="其他模型（不推荐用于此能力）">
                          {otherRemoteModels.map((name) => (
                            <option value={name} key={name}>
                              {name}
                            </option>
                          ))}
                        </optgroup>
                      )}
                    </>
                  ) : (
                    builtInModels.map((name) => (
                      <option value={name} key={name}>
                        {name}
                      </option>
                    ))
                  )}
                  {!listedModel && (
                    <option value={model} key={`current-${model}`}>
                      {model || "自定义模型"}
                    </option>
                  )}
                </select>
                <Button
                  variant="quiet"
                  disabled={backendLocked}
                  onClick={() => setAdvancedOpen(true)}
                >
                  高级
                </Button>
              </div>
            </Field>
          )}

          {currentModelMissing && (
            <p className="em-note warn" role="status">
              当前模型「{model}」不在服务商实时列表中；请确认它仍然可用，或改选其他模型。
            </p>
          )}

          {capability?.supportsModelList ? (
            <div className="em-field-row">
              <div>
                <p className="em-hint">
                  可用模型以服务商当前返回为准；拉取不会自动保存。
                </p>
                {!savedCredentials && !enteredApiKey && (
                  <p className="em-hint">先填写 API Key，才能拉取可用模型。</p>
                )}
              </div>
              <Button
                variant="quiet"
                disabled={!canFetchModels}
                onClick={() => void fetchModels()}
              >
                {loadingModels
                  ? "正在拉取…"
                  : remoteModels
                    ? "重新拉取模型"
                    : "拉取可用模型"}
              </Button>
            </div>
          ) : (
            <p className="em-hint">
              该服务商不提供模型列表接口，只能使用内置推荐模型或手动模型 ID。
            </p>
          )}

          <Collapsible
            title="高级设置"
            open={advancedOpen}
            onToggle={setAdvancedOpen}
          >
            {manualModelField && (
              <Field
                label="手动模型 ID"
                hint="普通用户保持推荐模型即可；只有明确需要时再改。"
              >
                <input
                  className="em-input"
                  value={model}
                  disabled={backendLocked}
                  onChange={(event) => changeModel(event.target.value)}
                />
              </Field>
            )}
            {capability?.protocolHint && (
              <p className="em-hint">{capability.protocolHint}</p>
            )}
            {capability?.note && <p className="em-note">{capability.note}</p>}
          </Collapsible>

          <div className="em-provider-credentials">
            {savedCredentials && (
              <div className="em-note ok" role="status">
                <div className="em-card-foot">
                  <span>
                    <strong>已保存 {fields.length} 项凭据</strong>
                    <br />
                    已保存在 macOS 钥匙串，默认不回显。
                  </span>
                  {onRevealCredentials && !editingCredentials && (
                    <Button
                      variant="quiet"
                      disabled={backendLocked}
                      onClick={() => {
                        if (showingSaved) {
                          hideSavedCredentials();
                        } else {
                          void revealSavedCredentials();
                        }
                      }}
                    >
                      {showingSaved ? "隐藏" : "显示"}
                    </Button>
                  )}
                  {!editingCredentials && (
                    <Button
                      variant="quiet"
                      disabled={backendLocked}
                      onClick={beginCredentialEdit}
                    >
                      修改
                    </Button>
                  )}
                </div>
              </div>
            )}

            {savedCredentials && showingSaved && !editingCredentials &&
              fields.map((field) => (
                <Field label={field.label} key={field.key}>
                  <input
                    className="em-input"
                    type="text"
                    readOnly
                    value={credentials[field.key] ?? ""}
                  />
                </Field>
              ))}

            {(!savedCredentials || editingCredentials) &&
              fields.map((field) => (
                <Field
                  label={field.label}
                  hint={
                    savedCredentials
                      ? "正在修改。为避免新旧凭据混用，请重新填写该服务商的全部凭据。"
                      : field.help
                  }
                  key={field.key}
                >
                  <div className="em-field-row">
                    <input
                      className="em-input"
                      type={field.secret && !revealed[field.key] ? "password" : "text"}
                      autoComplete="new-password"
                      value={credentials[field.key] ?? ""}
                      placeholder={field.placeholder}
                      disabled={backendLocked}
                      onChange={(event) => {
                        setCredentials((current) => ({
                          ...current,
                          [field.key]: event.target.value,
                        }));
                      }}
                    />
                    {field.secret && (
                      <Button
                        variant="quiet"
                        disabled={backendLocked}
                        onClick={() =>
                          setRevealed((current) => ({
                            ...current,
                            [field.key]: !current[field.key],
                          }))
                        }
                      >
                        {revealed[field.key] ? "隐藏" : "显示"}
                      </Button>
                    )}
                  </div>
                </Field>
              ))}
            {savedCredentials && editingCredentials && (
              <Button
                variant="quiet"
                disabled={backendLocked}
                onClick={cancelCredentialEdit}
              >
                取消修改
              </Button>
            )}
            <div className="em-card-actions">
              {savedCredentials && (
                <Button
                  variant="danger"
                  disabled={backendLocked}
                  onClick={() => void clearCredentials()}
                >
                  清除凭据
                </Button>
              )}
              {canReuse && (
                <Button
                  variant="quiet"
                  disabled={backendLocked}
                  onClick={() => void reuse()}
                >
                  复用已配置凭据
                </Button>
              )}
            </div>
          </div>

          <label className="em-check-row">
            <input
              type="checkbox"
              checked={consent}
              disabled={backendLocked}
              onChange={(event) => setConsent(event.target.checked)}
            />
            <span>
              {target === "analysis"
                ? `我已了解：启用后会把逐字稿全文和现有结构化分析发送给 ${provider.label}。`
                : `我已了解：转写时会把原始音频上传给 ${provider.label}。`}
            </span>
          </label>

          <div className="em-card-foot">
            <Button
              variant="primary"
              disabled={!canSubmit}
              onClick={() => void submitAndVerify()}
            >
              {verifying ? "正在验证当前模型…" : actionLabel}
            </Button>
            <Button
              variant="quiet"
              disabled={!canSubmit}
              onClick={() => void submit()}
            >
              仅保存
            </Button>
            {savedConfigVerified && (
              <Button
                variant="quiet"
                disabled={backendLocked}
                onClick={() => void verifySaved()}
              >
                {verifying ? "正在验证当前模型…" : "验证连通"}
              </Button>
            )}
          </div>
          <p className="em-hint">
            {target === "analysis"
              ? "验证会发送一次固定的测试消息（不含任何录音或笔记内容）。"
              : "验证会发送一段约 1 秒的静音测试音。"}
          </p>
        </>
      )}

      <ErrorNote error={errorNote} />
    </>
  );
}

function eligibleProvider(
  providers: ProviderDefinition[],
  providerId: string,
): ProviderDefinition | undefined {
  return providers.find((provider) => provider.id === providerId);
}

function capabilityFor(
  provider: ProviderDefinition | null | undefined,
  target: ProviderTarget,
): ProviderCapability | null {
  if (!provider) return null;
  return target === "analysis" ? provider.analysis : provider.transcription;
}

function storedBaseUrl(settings: ExternalAiSettings, target: ProviderTarget): string {
  return target === "analysis" ? settings.baseUrl : settings.transcriptionBaseUrl;
}

function storedModel(settings: ExternalAiSettings, target: ProviderTarget): string {
  return target === "analysis" ? settings.model : settings.transcriptionModel;
}

function storedConsent(settings: ExternalAiSettings, target: ProviderTarget): string | null {
  return target === "analysis" ? settings.privacyConsentAt : settings.audioUploadConsentAt;
}

function storedHasCredentials(settings: ExternalAiSettings, target: ProviderTarget): boolean {
  return target === "analysis" ? settings.hasApiKey : settings.transcriptionHasApiKey;
}
