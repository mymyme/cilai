import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { CapabilityCreationProject } from "./generated/protocol";
import { invoke } from "./host";
import { useModal } from "./use-modal";
import "./confirm-dialog.css";
import {PersonalApplicationData} from "./personal-application-data";

type PersonalState = { project: CapabilityCreationProject; enabled: boolean | null };
type Action = "enable" | "disable" | "uninstall";

export function PersonalCapabilityManager(props: {
  projectId: string;
  onClose: () => void;
  onChanged: (state: PersonalState, versionChanged?:boolean) => void;
  onImportUpgrade?: (project: CapabilityCreationProject) => void;
}) {
  const onChanged = useRef(props.onChanged);
  useLayoutEffect(() => { onChanged.current = props.onChanged; });
  const [state, setState] = useState<PersonalState | null>(null);
  const [busy, setBusy] = useState(true);
  const [dataBusy,setDataBusy] = useState(false);
  const [error, setError] = useState("");
  const [confirm, setConfirm] = useState(false);
  const modal = useModal<HTMLDivElement>({ onClose: () => confirm ? setConfirm(false) : props.onClose(), busy:busy||dataBusy });
  const load = useCallback(async (versionChanged=false) => {
    setBusy(true); setError("");
    try {
      const latest = await invoke<PersonalState>("personal_capability_status", { creationProjectId: props.projectId });
      setState(latest); if(versionChanged)onChanged.current(latest,true);else onChanged.current(latest);
    }
    catch { setError("读取失败，请刷新后重试。"); }
    finally { setBusy(false); }
  }, [props.projectId]);
  useEffect(() => { void load(); }, [load]);

  const apply = async (action: Action) => {
    if (!state?.project.packageDigest || busy) return;
    setBusy(true); setError("");
    try {
      const next = await invoke<PersonalState>("personal_capability_manage", { request: {
        creationProjectId: props.projectId,
        packageDigest: state.project.packageDigest,
        expectedRevision: state.project.stateRevision,
        action,
        clientOperationId: crypto.randomUUID(),
      } });
      setState(next); setConfirm(false); props.onChanged(next);
      if (action === "uninstall") props.onClose();
    } catch {
      setState(null); setConfirm(false);
      setError("操作未确认，请刷新状态后重试。");
    } finally { setBusy(false); }
  };

  return <div role="dialog" aria-modal="true" aria-label="管理个人能力" className="agent-picker-backdrop" ref={modal}>
    <section className="artifact-review">
      <header><h3>{state?.project.title ?? "个人能力"}</h3><button type="button" disabled={busy||dataBusy} onClick={props.onClose}>关闭</button></header>
      {busy ? <p role="status">正在处理…</p> : null}
      {error ? <p role="alert">{error}</p> : null}
      {state ? <>
        <p>{state.enabled === null ? "未安装" : state.enabled ? "已启用" : "已停用"}</p>
        {confirm ? <>
          <p>卸载后无法运行，创建项目和个人数据会保留。</p>
          <footer className="confirm-actions"><button type="button" disabled={busy||dataBusy} onClick={() => setConfirm(false)}>取消</button><button className="danger" type="button" disabled={busy||dataBusy} onClick={() => void apply("uninstall")}>确认卸载</button></footer>
        </> : state.enabled !== null ? <footer className="confirm-actions">
          {props.onImportUpgrade ? <button type="button" disabled={busy||dataBusy} onClick={() => props.onImportUpgrade?.(state.project)}>导入升级</button> : null}
          <button type="button" disabled={busy||dataBusy} onClick={() => void apply(state.enabled ? "disable" : "enable")}>{state.enabled ? "停用" : "启用"}</button>
          <button type="button" disabled={busy||dataBusy} onClick={() => setConfirm(true)}>卸载</button>
        </footer> : null}
        <PersonalApplicationData project={state.project} disabled={busy||confirm} onBusy={setDataBusy} onChanged={()=>load(true)}/>
      </> : !busy ? <footer className="confirm-actions"><button type="button" onClick={() => void load()}>刷新</button></footer> : null}
    </section>
  </div>;
}
