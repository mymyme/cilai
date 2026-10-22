import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { aiErrorMessage } from "./ai-runtime";
import { ConfirmDialog } from "./confirm-dialog";
import { createPortal } from "./page-portal";
import { Select } from "./select";
import { useModal } from "./use-modal";
import type { ModelConnection, ModelDescriptor, ModelRuntimeKind, ModelValidationLayer, ModelValidationRecord } from "./generated/protocol";
import { loadConnectionModels, loadConnectionValidations, loadModelConnections, refreshConnectionModels, saveConnectionModel, validateModelConnection } from "./model-connections";
import "./model-connection-models.css";

const modalityNames: Record<string, string> = { text: "文本", image: "图片", audio: "音频", video: "视频" };
const layers: Record<ModelValidationLayer, string> = { configuration: "配置", network: "网络", authentication: "认证", protocol: "Responses 协议", catalog: "目录", inference: "推理", streaming: "流式", tools: "工具" };
const metadataValidation = (layer: ModelValidationLayer) => ["configuration", "network", "catalog"].includes(layer);
type Editor = { model: ModelDescriptor | null };

export function ConnectionModels(props: { connection: ModelConnection; disabled: boolean; onChanged: () => Promise<void> }) {
  const [models, setModels] = useState<ModelDescriptor[]>([]);
  const [validations, setValidations] = useState<ModelValidationRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadedScope, setLoadedScope] = useState("");
  const scopeKey = `${props.connection.profileId}:${props.connection.connectionId}:${props.connection.configRevision}:${props.connection.authGeneration}`;
  const [query, setQuery] = useState("");
  const [editor, setEditor] = useState<Editor | null>(null);
  const [catalog, setCatalog] = useState(false);
  const [validation, setValidation] = useState(false);
  const [validationModelId, setValidationModelId] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [readError, setReadError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    void Promise.all([loadConnectionModels(props.connection.connectionId), loadConnectionValidations(props.connection.connectionId)])
      .then(([items, checks]) => { if (!cancelled) { setModels(items); setValidations(checks); setLoadedScope(scopeKey); setReadError(null); } })
      .catch((reason) => { if (!cancelled) { setLoadedScope(""); setReadError(aiErrorMessage(reason)); } })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [props.connection.connectionId, props.connection.configRevision, props.connection.authGeneration, scopeKey, revision]);
  const snapshotReady = loadedScope === scopeKey;
  const disabled = busy || props.disabled || loading || !snapshotReady;
  const openEditor = (model: ModelDescriptor | null) => { setError(null); setCatalog(false); setEditor({ model }); };
  const openValidation = (modelId = "") => { setError(null); setValidationModelId(modelId); setValidation(true); };
  const refresh = async () => {
    if (disabled) return;
    setBusy(true); setError(null);
    try { await refreshConnectionModels(props.connection.connectionId); setRevision((value) => value + 1); await props.onChanged(); }
    catch (reason) { setError(aiErrorMessage(reason)); }
    finally { setBusy(false); }
  };
  return <section className="model-library" aria-label="连接模型">
    <header className="model-library-heading"><h2>模型 <span>{snapshotReady ? models.length : "—"}</span></h2><div className="model-library-actions">
      <button type="button" disabled={disabled || loading} onClick={() => openValidation()}><LibraryIcon kind="check"/>验证连接</button>
      <button type="button" disabled={disabled || loading} onClick={() => { setError(null); setCatalog(true); }}><LibraryIcon kind="catalog"/>查看目录</button>
      <button type="button" disabled={disabled || loading} onClick={() => openEditor(null)}><LibraryIcon kind="plus"/>添加模型</button>
    </div></header>
    <Search value={query} onChange={setQuery}/>
    {loading ? <p className="model-library-empty" role="status">正在读取…</p> : snapshotReady ? <ModelGroups models={filterModels(models, query)} disabled={disabled}
      onEdit={openEditor} onValidate={openValidation} empty={query ? "没有匹配的模型" : "尚无模型记录"}/> : null}
    {(error || readError) && !catalog && !editor && !validation ? <p role="alert" className="model-library-error">{error ?? readError}{!snapshotReady ? <button type="button" disabled={busy || props.disabled || loading} onClick={() => setRevision((value) => value + 1)}>重试读取</button> : null}</p> : null}
    {catalog ? <CatalogPanel models={snapshotReady ? models : null} busy={busy} disabled={disabled} loading={loading} canRefresh={props.connection.config.enabled} error={error ?? readError}
      onClose={() => setCatalog(false)} onRefresh={() => void refresh()} onEdit={openEditor}
      canRetryRead={!busy && !props.disabled && !loading} onRetryRead={() => setRevision((value) => value + 1)}/> : null}
    {editor ? <ModelEditor key={editor.model?.descriptorId ?? "new"} model={editor.model} models={models} connectionId={props.connection.connectionId}
      disabled={props.disabled || !snapshotReady} onClose={() => setEditor(null)} onSaved={async (warning) => {
        setEditor(null); setRevision((value) => value + 1); setError(warning); await props.onChanged();
      }}/> : null}
    {validation ? <ValidationPanel connection={props.connection} models={snapshotReady ? models : []} validations={snapshotReady ? validations : []} disabled={props.disabled || !snapshotReady}
      sourceError={readError} initialModelId={validationModelId} onClose={() => setValidation(false)} onChanged={async () => { setRevision((value) => value + 1); await props.onChanged(); }}/> : null}
  </section>;
}

function filterModels(models: ModelDescriptor[], query: string) {
  const value = query.trim().toLocaleLowerCase();
  return models.filter((model) => `${model.modelId} ${model.displayName}`.toLocaleLowerCase().includes(value));
}
function Search(props: { value: string; onChange: (value: string) => void; autoFocus?: boolean }) {
  return <label className="model-library-search"><LibraryIcon kind="search"/><input autoFocus={props.autoFocus} aria-label="搜索模型" placeholder="搜索模型…" type="search" value={props.value} onChange={(event) => props.onChange(event.target.value)}/></label>;
}
function ModelGroups(props: { models: ModelDescriptor[]; disabled: boolean; onEdit: (model: ModelDescriptor) => void; onValidate?: (modelId: string) => void; empty: string }) {
  const groups = new Map<string, ModelDescriptor[]>();
  for (const model of props.models) {
    const prefix = model.modelId.includes("/") ? model.modelId.split("/", 1)[0] : "未分组";
    groups.set(prefix, [...(groups.get(prefix) ?? []), model]);
  }
  if (!groups.size) return <p className="model-library-empty">{props.empty}</p>;
  return <div className="model-library-groups">{[...groups].sort(([a], [b]) => a.localeCompare(b)).map(([prefix, entries]) => <details key={prefix} open className="model-library-group">
    <summary><LibraryIcon kind="chevron"/><strong>{prefix}</strong><span>{entries.length}</span></summary>
    <ul>{entries.map((model) => <li key={model.descriptorId}>
      <div className="model-library-model-icon"><LibraryIcon kind="model"/></div>
      <div className="model-library-model-name"><strong>{model.displayName}</strong><code>{model.modelId}</code></div>
      <div className="model-library-badges"><span title={model.source === "user_declared" ? "手动填写的模型信息，不代表已验证" : model.source === "runtime_catalog" ? "运行时目录提供的信息，不代表调用权限" : model.source === "observed" ? "已观测元数据，能力以对应验证回执为准" : "服务目录提供的信息，不代表调用权限"}>{model.source === "user_declared" ? "手动声明" : model.source === "observed" ? "观测记录" : "目录声明"}</span>
        {model.inputModalities.map((item) => <span key={item} title={`声明输入：${modalityNames[item] ?? item}`}>{modalityNames[item] ?? item}</span>)}
      </div>
      <div className="model-library-row-actions">{props.onValidate ? <button type="button" title="验证" aria-label={`验证 ${model.displayName}`} disabled={props.disabled} onClick={() => props.onValidate?.(model.modelId)}><LibraryIcon kind="check"/></button> : <small>已记录</small>}
        <button type="button" title="编辑" aria-label={`编辑 ${model.displayName}`} disabled={props.disabled} onClick={() => props.onEdit(model)}><LibraryIcon kind="edit"/></button></div>
    </li>)}</ul>
  </details>)}</div>;
}
function LibraryModal(props: { title: string; busy?: boolean; active?: boolean; onClose: () => void; children: ReactNode; wide?: boolean }) {
  const titleId = useId();
  const ref = useModal<HTMLDivElement>({ onClose: props.onClose, busy: props.busy, active: props.active });
  return createPortal(<div className="model-library-backdrop" ref={ref} role="dialog" aria-modal="true" aria-labelledby={titleId}
    onMouseDown={(event) => { if (event.target === event.currentTarget && !props.busy && props.active !== false) props.onClose(); }}>
    <section className={`model-library-dialog${props.wide ? " model-library-dialog-wide" : ""}`}><header><h3 id={titleId}>{props.title}</h3><button aria-label="关闭" type="button" disabled={props.busy} onClick={props.onClose}><LibraryIcon kind="close"/></button></header>{props.children}</section>
  </div>, document.body);
}
function CatalogPanel(props: { models: ModelDescriptor[] | null; busy: boolean; disabled: boolean; loading: boolean; canRefresh: boolean; error: string | null; canRetryRead: boolean; onClose: () => void; onRefresh: () => void; onRetryRead: () => void; onEdit: (model: ModelDescriptor) => void }) {
  const [query, setQuery] = useState("");
  const [modality, setModality] = useState("");
  const filtered = filterModels(props.models ?? [], query).filter((model) => !modality || model.inputModalities.includes(modality));
  return <LibraryModal title="模型目录" busy={props.busy} onClose={props.onClose} wide>
    <div className="model-library-dialog-body"><div className="model-library-catalog-toolbar"><span>{props.models ? `${props.models.length} 个已记录模型` : props.loading ? "正在读取…" : "目录不可用"}</span><button type="button" disabled={props.disabled || !props.canRefresh} onClick={props.onRefresh}><LibraryIcon kind="refresh"/>读取目录</button></div>
      <Search value={query} onChange={setQuery} autoFocus/><div className="model-library-filters" role="group" aria-label="目录输入类型筛选">{["", ...Object.keys(modalityNames)].map((value) => <button key={value} type="button" aria-pressed={modality === value} onClick={() => setModality(value)}>{value ? modalityNames[value] : "全部"}<span>{props.models ? props.models.filter((model) => !value || model.inputModalities.includes(value)).length : "—"}</span></button>)}</div>
      {props.error ? <p className="model-library-error" role="alert">{props.error}{!props.models ? <button type="button" disabled={!props.canRetryRead} onClick={props.onRetryRead}>重试读取</button> : null}</p> : null}
      {props.models ? <ModelGroups models={filtered} disabled={props.disabled} onEdit={props.onEdit} empty="没有匹配的模型"/> : null}
    </div><footer><small>目录声明不代表调用权限。</small><button type="button" disabled={props.busy} onClick={props.onClose}>完成</button></footer>
  </LibraryModal>;
}
function ModelEditor(props: { model: ModelDescriptor | null; models: ModelDescriptor[]; connectionId: string; disabled: boolean; onClose: () => void; onSaved: (warning: string | null) => Promise<void> }) {
  const [modelId, setModelId] = useState(props.model?.modelId ?? "");
  const [name, setName] = useState(props.model?.displayName ?? "");
  const [context, setContext] = useState(props.model?.contextWindowTokens?.toString() ?? "");
  const [modalities, setModalities] = useState<string[]>(props.model?.inputModalities ?? []);
  const [efforts, setEfforts] = useState(props.model?.supportedReasoningEfforts.join(",") ?? "");
  const [defaultEffort, setDefaultEffort] = useState(props.model?.defaultReasoningEffort ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const disabled = busy || props.disabled;
  const save = async () => {
    if (disabled || !modelId.trim()) return;
    if (!props.model && props.models.some((item) => item.modelId === modelId.trim())) { setError("模型已记录，请编辑已有记录。"); return; }
    setBusy(true); setError(null);
    try {
      const receipt = await saveConnectionModel(props.connectionId, modelId.trim(), name.trim() || modelId.trim(), {
        contextWindowTokens: context ? Number(context) : null, inputModalities: modalities, defaultReasoningEffort: defaultEffort.trim() || null,
        supportedReasoningEfforts: efforts.split(",").map((value) => value.trim()).filter(Boolean),
      }, props.model?.revision ?? null);
      await props.onSaved(receipt.catalogRefreshError ? "模型已保存，目录刷新尚未完成。请重新读取目录。" : null);
    } catch (reason) { setError(aiErrorMessage(reason)); }
    finally { setBusy(false); }
  };
  return <LibraryModal title={props.model ? "编辑模型" : "添加模型"} busy={busy} onClose={props.onClose}>
    <form aria-label={props.model ? "编辑模型" : "添加模型"} onSubmit={(event) => { event.preventDefault(); void save(); }}>
      <div className="model-library-dialog-body"><fieldset disabled={disabled} className="model-library-editor">
        <label>模型 ID<input autoFocus={!props.model} required readOnly={Boolean(props.model)} placeholder="例如 nvidia/model-id:free" value={modelId} onChange={(event) => setModelId(event.target.value)}/></label>
        <label>显示名称<input autoFocus={Boolean(props.model)} placeholder="可选" value={name} onChange={(event) => setName(event.target.value)}/></label>
        <details className="model-library-more" open={Boolean(props.model)}><summary>更多设置<LibraryIcon kind="chevron"/></summary>
          <div className="model-library-inputs"><label>上下文 Token<input type="number" min={1} max={1_000_000_000} step={1} placeholder="未知" value={context} onChange={(event) => setContext(event.target.value)}/></label>
          <div role="group" aria-label="声明输入类型" className="model-library-modalities">{Object.entries(modalityNames).map(([value, label]) => <label key={value}><input type="checkbox" checked={modalities.includes(value)} onChange={(event) => setModalities((current) => event.target.checked ? [...current, value] : current.filter((item) => item !== value))}/>{label}</label>)}</div>
          <label>思考档位<input placeholder="用英文逗号分隔" value={efforts} onChange={(event) => setEfforts(event.target.value)}/></label><label>默认思考档位<input placeholder="未知" value={defaultEffort} onChange={(event) => setDefaultEffort(event.target.value)}/></label></div>
        </details><small>留空表示未知；声明不代表调用或工具能力已验证。</small>
        {error ? <p role="alert" className="model-library-error">{error}</p> : null}
      </fieldset></div><footer><button type="button" disabled={busy} onClick={props.onClose}>取消</button><button type="submit" className="primary" disabled={disabled || !modelId.trim()}>{busy ? "保存中…" : props.model ? "保存模型" : "登记模型"}</button></footer>
    </form>
  </LibraryModal>;
}
type ValidationTarget = { profileId: string; connectionId: string; configRevision: number; authGeneration: number; runtimeKind: ModelRuntimeKind; modelId: string; connectionName: string; metadataOnly: boolean };
type ValidationStep = { state: "queued" | "running" | "passed" | "failed" | "not_run" | "unconfirmed"; reason?: string | null };
type ValidationRound = { target: ValidationTarget; steps: Partial<Record<ModelValidationLayer, ValidationStep>> };
const validationOrder = Object.keys(layers) as ModelValidationLayer[];
const ordinaryValidationFailures = new Set([
  "model_validation_configuration_invalid", "model_validation_network_http_rejected", "model_validation_authentication_failed",
  "model_validation_authentication_not_established", "model_validation_authentication_response_invalid", "model_validation_catalog_failed",
  "model_validation_model_not_in_catalog", "model_validation_runtime_failed", "model_validation_empty_response", "model_validation_tool_result_invalid",
  "model_validation_inference_nonce_mismatch", "model_validation_streaming_delta_missing", "model_validation_tool_call_missing",
]);
const stepLabels: Record<ValidationStep["state"], string> = { queued: "待执行", running: "进行中", passed: "通过", failed: "失败", not_run: "未执行", unconfirmed: "结果未确认" };
function validationGeneration(connection: ModelConnection, kind: ModelRuntimeKind) {
  return connection.config.kind === "chatgpt" ? connection.authScopes.find((scope) => scope.runtimeKind === kind && scope.state === "active")?.authGeneration ?? 0 : connection.authGeneration;
}
function validationTargetKey(target: ValidationTarget) {
  return `${target.profileId}:${target.connectionId}:${target.configRevision}:${target.authGeneration}:${target.runtimeKind}:${target.modelId}`;
}
function matchesValidationConnection(connection: ModelConnection | undefined, target: ValidationTarget) {
  return connection && !connection.deleted && connection.config.enabled && connection.profileId === target.profileId
    && connection.connectionId === target.connectionId && connection.configRevision === target.configRevision
    && validationGeneration(connection, target.runtimeKind) === target.authGeneration;
}
function validValidationReceipt(record: ModelValidationRecord, target: ValidationTarget, layer: ModelValidationLayer) {
  return record && record.connectionId === target.connectionId && record.runtimeKind === target.runtimeKind && record.modelId === target.modelId
    && record.layer === layer && record.configRevision === target.configRevision && record.authGeneration === (metadataValidation(layer) ? 0 : target.authGeneration)
    && typeof record.validationId === "string" && record.validationId.length > 0
    && Number.isSafeInteger(record.completionRevision) && record.completionRevision !== null && record.completionRevision > 0
    && (record.outcome === "passed" || record.outcome === "failed") && (record.reason === null || typeof record.reason === "string")
    && (record.runtimeVersion === null || typeof record.runtimeVersion === "string") && (record.serviceVersion === null || typeof record.serviceVersion === "string")
    && Number.isSafeInteger(record.updatedAt) && record.updatedAt >= 0;
}
function ValidationPanel(props: { connection: ModelConnection; models: ModelDescriptor[]; validations: ModelValidationRecord[]; initialModelId: string; sourceError: string | null; disabled: boolean; onClose: () => void; onChanged: () => Promise<void> }) {
  const [modelId, setModelId] = useState(props.initialModelId);
  const [kind, setKind] = useState<ModelRuntimeKind>("workbench");
  const [confirmation, setConfirmation] = useState<ValidationTarget | null>(null);
  const [round, setRound] = useState<ValidationRound | null>(null);
  const [busy, setBusy] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshError, setRefreshError] = useState<string | null>(null);
  const inFlight = useRef(false);
  const mounted = useRef(true);
  const latest = useRef({ ...props, kind, modelId });
  latest.current = { ...props, kind, modelId };
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const authGeneration = validationGeneration(props.connection, kind);
  const target: ValidationTarget = { profileId: props.connection.profileId, connectionId: props.connection.connectionId, configRevision: props.connection.configRevision,
    authGeneration, runtimeKind: kind, modelId, connectionName: props.connection.config.name, metadataOnly: props.connection.config.kind === "chatgpt" && authGeneration === 0 };
  const targetKey = validationTargetKey(target);
  useEffect(() => { setConfirmation(null); if (!inFlight.current) { setRound(null); setError(null); setRefreshError(null); } }, [targetKey]);
  const isCurrent = (frozen: ValidationTarget) => {
    const current = latest.current;
    return mounted.current && !current.disabled && !current.sourceError && current.kind === frozen.runtimeKind && current.modelId === frozen.modelId
      && matchesValidationConnection(current.connection, frozen) && current.models.some((model) => model.modelId === frozen.modelId && model.connectionId === frozen.connectionId);
  };
  const updateStep = (layer: ModelValidationLayer, step: ValidationStep) => {
    if (mounted.current) setRound((current) => current ? { ...current, steps: { ...current.steps, [layer]: step } } : current);
  };
  const stopRemaining = () => {
    if (mounted.current) setRound((current) => current ? { ...current, steps: Object.fromEntries(validationOrder.map((layer) => {
      const step = current.steps[layer]; return [layer, step?.state === "queued" ? { state: "not_run" } : step];
    })) } : current);
  };
  const reread = async () => {
    if (inFlight.current || refreshing) return;
    setRefreshing(true); setRefreshError(null);
    try { await props.onChanged(); } catch (reason) { if (mounted.current) setRefreshError(aiErrorMessage(reason)); }
    finally { if (mounted.current) setRefreshing(false); }
  };
  const validateAll = async () => {
    if (!confirmation || inFlight.current) return;
    const frozen = confirmation;
    setConfirmation(null);
    if (!isCurrent(frozen)) { setError("连接、模型或授权已变化，请重新确认。"); return; }
    inFlight.current = true; setBusy(true); setError(null); setRefreshError(null);
    setRound({ target: frozen, steps: Object.fromEntries(validationOrder.map((layer) => [layer, frozen.metadataOnly && !metadataValidation(layer)
      ? { state: "not_run", reason: "此用途未授权" } : { state: "queued" }])) });
    const seen = new Set(latest.current.validations.map((record) => record.validationId));
    let active: ModelValidationLayer | null = null;
    let submitted = false;
    try {
      for (const layer of validationOrder) {
        if (frozen.metadataOnly && !metadataValidation(layer)) continue;
        active = layer; submitted = false;
        if (!isCurrent(frozen)) throw Error("连接、模型或授权已变化，已停止后续验证。");
        updateStep(layer, { state: "running" });
        const [connections, models] = await Promise.all([loadModelConnections(), loadConnectionModels(frozen.connectionId)]);
        if (!isCurrent(frozen) || !matchesValidationConnection(connections.find((connection) => connection.connectionId === frozen.connectionId), frozen)) throw Error("连接或授权已失效，已停止后续验证。");
        if (!models.some((model) => model.connectionId === frozen.connectionId && model.modelId === frozen.modelId)) throw Error("所选模型已移除，已停止后续验证。");
        submitted = true;
        const receipt = await validateModelConnection({ runtimeKind: frozen.runtimeKind, connectionId: frozen.connectionId, modelId: frozen.modelId, layer,
          expectedConfigRevision: frozen.configRevision, expectedAuthGeneration: metadataValidation(layer) ? 0 : frozen.authGeneration });
        if (!validValidationReceipt(receipt, frozen, layer) || seen.has(receipt.validationId)) throw Error("验证回执不匹配，本项结果未确认。");
        seen.add(receipt.validationId);
        updateStep(layer, { state: receipt.outcome as "passed" | "failed", reason: receipt.outcome === "failed" ? aiErrorMessage(receipt.reason ?? "model_validation_failed") : null });
        active = null; submitted = false;
        if (!isCurrent(frozen)) {
          if (mounted.current) setError("连接、模型或授权已变化，已停止后续验证。");
          stopRemaining(); break;
        }
        if (receipt.outcome === "failed" && !ordinaryValidationFailures.has(receipt.reason ?? "")) {
          if (mounted.current) setError(`已停止后续验证：${aiErrorMessage(receipt.reason ?? "model_validation_failed")}`);
          stopRemaining(); break;
        }
      }
    } catch (reason) {
      if (active) updateStep(active, { state: submitted ? "unconfirmed" : "not_run", reason: aiErrorMessage(reason) });
      if (mounted.current) setError(aiErrorMessage(reason));
      stopRemaining();
    } finally {
      if (mounted.current) {
        try { await latest.current.onChanged(); } catch (reason) { if (mounted.current) setRefreshError(aiErrorMessage(reason)); }
      }
      inFlight.current = false;
      if (mounted.current) setBusy(false);
    }
  };
  const selected = props.models.some((model) => model.modelId === modelId && model.connectionId === props.connection.connectionId);
  const disabled = busy || props.disabled || !props.connection.config.enabled || props.connection.deleted || !selected;
  const visibleRound = round && validationTargetKey(round.target) === targetKey ? round : null;
  const unavailable = props.disabled && round && !busy ? "连接资料暂不可用，请重新读取。" : null;
  const history = props.validations.filter((record) => validValidationReceipt(record, target, record.layer))
    .sort((left, right) => left.completionRevision! - right.completionRevision!);
  return <><LibraryModal title="连接验证" busy={busy} active={!confirmation} onClose={props.onClose}>
    <form aria-label="连接验证" onSubmit={(event) => { event.preventDefault(); if (!disabled && !inFlight.current) setConfirmation({ ...target }); }}>
      <div className="model-library-dialog-body"><fieldset className="model-library-validation-form" disabled={busy || props.disabled}>
        <label>验证用途<Select autoFocus disabled={busy || props.disabled} ariaLabel="验证用途" value={kind} onChange={(value) => setKind(value as ModelRuntimeKind)} options={[{ value: "workbench", label: "主对话" }, { value: "application", label: "页面 AI" }]}/></label>
        <label>验证模型<Select disabled={busy || props.disabled} ariaLabel="验证模型" value={modelId} onChange={setModelId} options={[{ value: "", label: "未选择" }, ...props.models.map((model) => ({ value: model.modelId, label: model.displayName }))]}/></label>
        {target.metadataOnly ? <p className="model-library-note">此用途未授权，仅检查配置、网络和目录。</p> : null}
        <p className="model-library-note" role="status">{visibleRound ? "本轮结果" : "上次结果"}</p>
        <div className="model-library-validation" aria-live="polite">{validationOrder.map((layer) => {
          const step = visibleRound?.steps[layer];
          const previous = [...history].reverse().find((record) => record.layer === layer);
          return <span key={layer} data-layer={layer} data-outcome={step?.state ?? "unverified"}>{layers[layer]}<small>{step ? stepLabels[step.state] : previous?.outcome === "passed" ? "上次通过" : previous?.outcome === "failed" ? "上次失败" : "未验证"}</small>
            {step?.reason ? <small className="model-library-validation-reason">{step.reason}</small> : null}</span>;
        })}</div>{error || props.sourceError || refreshError || unavailable ? <p className="model-library-error" role="alert">{error ?? props.sourceError ?? refreshError ?? unavailable}</p> : null}
      </fieldset></div><footer>{refreshError || props.sourceError || (props.disabled && round) ? <button type="button" disabled={busy || refreshing} onClick={() => void reread()}>{refreshing ? "读取中…" : "重新读取"}</button> : null}<button type="button" disabled={busy} onClick={props.onClose}>关闭</button><button type="submit" className="primary" disabled={disabled}>{busy ? "验证中…" : "全部验证"}</button></footer>
    </form>
  </LibraryModal>{confirmation ? <ConfirmDialog title={confirmation.metadataOnly ? "验证配置？" : "验证模型调用？"} confirmLabel="全部验证" onCancel={() => setConfirmation(null)} onConfirm={() => void validateAll()}
    description={confirmation.metadataOnly ? `检查“${confirmation.connectionName}”的“${confirmation.modelId}”（${confirmation.runtimeKind === "application" ? "页面 AI" : "主对话"}）配置、网络和目录。此用途未授权，其余五项不执行；不调用模型，不发送用户历史。`
      : `使用“${confirmation.connectionName}”的“${confirmation.modelId}”（${confirmation.runtimeKind === "application" ? "页面 AI" : "主对话"}）依次验证配置、网络、认证、Responses 协议、目录、推理、流式和工具。将读取目录并发送少量隔离测试输入，工具验证仅使用隔离测试工具；不发送用户历史，可能产生服务费用。`}/> : null}</>;
}
function LibraryIcon(props: { kind: "plus" | "search" | "catalog" | "check" | "chevron" | "model" | "edit" | "close" | "refresh" }) {
  const paths = { plus: "M12 5v14M5 12h14", search: "m20 20-5-5M17 10a7 7 0 1 1-14 0 7 7 0 0 1 14 0", catalog: "M5 4h14v16H5zM8 8h8M8 12h8M8 16h5", check: "m5 12 4 4L19 6", chevron: "m6 9 6 6 6-6", model: "m12 3 9 5v8l-9 5-9-5V8zm0 10 9-5m-9 5L3 8m9 5v8", edit: "m15 4 5 5M4 20l5-1L20 8l-4-4L5 15z", close: "m6 6 12 12M6 18 18 6", refresh: "M20 7v5h-5M4 17v-5h5M6 7a7 7 0 0 1 12-1l2 3M4 15l2 3a7 7 0 0 0 12-1" };
  return <svg aria-hidden="true" viewBox="0 0 24 24"><path d={paths[props.kind]}/></svg>;
}
