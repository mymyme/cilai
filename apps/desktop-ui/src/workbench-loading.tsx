import cilaiSymbolUrl from "../../../assets/cilai-symbol-primary.svg";
import "./workbench-loading.css";

export function WorkbenchLoading({ title = "正在启动", detail = "正在准备词来工作台…" }: { title?: string; detail?: string }) {
  return <section aria-busy="true" aria-label={title} className="workbench-loading" role="status">
    <img alt="" aria-hidden="true" className="workbench-loading-mark" src={cilaiSymbolUrl}/>
    <h2>{title}</h2>
    {detail ? <p>{detail}</p> : null}
    <span aria-hidden="true" className="workbench-loading-spinner"/>
  </section>;
}
