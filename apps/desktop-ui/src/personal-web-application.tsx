import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "./host";
import { personalWebApplicationDocument } from "./personal-page-ai-sdk";
import { PersonalPageAiConfirmation, PersonalPageAiStatus, usePersonalPageAi } from "./personal-page-ai";

export function PersonalWebApplication({ projectId, packageDigest, html, title, active, onOpenModels }: { projectId: string; packageDigest: string; html: string; title: string; active: boolean; onOpenModels?: () => void }) {
  const frame = useRef<HTMLIFrameElement>(null);
  const channel = useMemo(() => crypto.randomUUID(), [projectId, packageDigest, html]);
  const source = useMemo(() => personalWebApplicationDocument(html, channel), [html, channel]);
  const [url, setUrl] = useState("");
  const [error, setError] = useState("");
  const pageAi = usePersonalPageAi({ frame, channel, url, active, enabled: true });
  useEffect(() => {
    let current = true;
    let documentUrl: string | undefined;
    setUrl("");
    setError("");
    const release = (value: string) => {
      void invoke("personal_document_release", { url: value }).catch(reason => console.error("应用页面释放失败", reason));
    };
    void invoke<string>("personal_document_create", { html: source, pageBinding: { creationProjectId: projectId, expectedPackageDigest: packageDigest } }).then(value => {
      if (!current) { release(value); return; }
      documentUrl = value;
      setUrl(value);
    }).catch(reason => { if (current) setError(String(reason)); });
    return () => { current = false; if (documentUrl) release(documentUrl); };
  }, [source, projectId, packageDigest]);
  return <section className="personal-application-surface" data-personal-project={projectId} hidden={!active}>
    <PersonalPageAiStatus error={pageAi.error} needsModel={pageAi.needsModel} onOpenModels={onOpenModels}/>
    {error ? <p role="alert">{error}</p> : !url ? <p role="status">正在打开…</p> :
    <iframe ref={frame} className="personal-capability-preview" sandbox="allow-scripts" src={url} title={title} hidden={!active}/>}
    {pageAi.pending ? <PersonalPageAiConfirmation key={pageAi.pending.writeRequest!.approval.confirmationId} pending={pageAi.pending} resolve={pageAi.resolve} active={active}/> : null}
  </section>;
}
