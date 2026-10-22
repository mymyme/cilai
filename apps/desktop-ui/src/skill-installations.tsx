import { useEffect, useRef, useState } from "react";
import { rpc } from "./app-server-rpc";
import { describeServerError } from "./server-error";
import type { SkillRunInstallation, SkillRunListResult } from "./generated/protocol";
import { SkillIcon } from "./icons";

export function useSkillInstallations(runIds: string[], profile: string | null | undefined) {
  const key = JSON.stringify(runIds);
  const [installations, setInstallations] = useState<Record<string, SkillRunInstallation>>({});
  const [error, setError] = useState<string | null>(null);
  const epoch = useRef(0);
  useEffect(() => {
    let active = true;
    const generation = ++epoch.current;
    setInstallations({});
    setError(null);
    const ids = JSON.parse(key) as string[];
    let timer: ReturnType<typeof setTimeout> | undefined;
    const load = async () => {
      if (!ids.length) return;
      try {
        const result = await rpc<SkillRunListResult>("skill/runList", { runIds: ids });
        if (!active || epoch.current !== generation) return;
        setInstallations(Object.fromEntries(result.installations.map((item) => [item.runId, item])));
        setError(null);
      } catch (failure) {
        if (!active) return;
        setError(describeServerError(failure));
      }
      if (active) timer = setTimeout(() => void load(), 2000);
    };
    void load();
    return () => { active = false; epoch.current += 1; clearTimeout(timer); };
  }, [key, profile]);
  const retry = async (runId: string) => {
    const generation = epoch.current;
    const result = await rpc<SkillRunInstallation>("skill/runRetry", { runId });
    if (epoch.current === generation) setInstallations((current) => ({ ...current, [runId]: result }));
  };
  return { installations, error, retry };
}

export function SkillInstallationStatus(props: {
  installation: SkillRunInstallation;
  onRetry: () => Promise<void>;
  onView?: (id: string) => void;
  onTry: (id: string, name: string) => void;
}) {
  const item = props.installation;
  const [busy, setBusy] = useState(false);
  const [requestError, setRequestError] = useState<string | null>(null);
  if (item.status === "ignored") return null;
  const reason = item.errorDetail ?? (item.errorReason ? describeServerError(JSON.stringify({ reason: item.errorReason })) : null);
  const label = item.status === "installed" ? `已添加 · ${item.name ?? "技能"}`
    : item.status === "pending" ? "正在添加技能…"
    : item.status === "historical" ? "技能待添加"
    : item.status === "deleted" ? "技能已删除" : "添加失败";
  const retry = async () => {
    setBusy(true); setRequestError(null);
    try { await props.onRetry(); }
    catch (error) { setRequestError(describeServerError(error)); }
    finally { setBusy(false); }
  };
  return <footer className="ai-skill-draft-action">
    <span><SkillIcon/>{label}</span>
    {item.status === "installed" && item.skillId ? <>
      {props.onView ? <button type="button" onClick={() => props.onView?.(item.skillId!)}>查看</button> : null}
      <button type="button" onClick={() => props.onTry(item.skillId!, item.name ?? "技能")}>试用</button>
    </> : null}
    {item.retryable ? <button type="button" disabled={busy} onClick={() => void retry()}>{busy ? "正在添加…" : item.status === "historical" ? "添加" : "重试"}</button> : null}
    {reason || requestError ? <details><summary>详情</summary><p>{requestError ?? reason}</p>{item.status === "invalid" ? <p>请在对话中修正上述问题后重新交付。</p> : null}</details> : null}
  </footer>;
}
