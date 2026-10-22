import type { ReactNode } from "react";

export function RuntimeStatusCard(props: {
  title: string;
  description: ReactNode;
  icon: ReactNode;
  status: string;
  tone: "success" | "warning" | "error" | "neutral";
  hint: ReactNode;
  actions: ReactNode;
  children?: ReactNode;
  className?: string;
}) {
  return <section aria-label={props.title} className={`runtime-status-card ${props.className ?? ""}`} data-tone={props.tone} data-ready={props.tone === "success"}>
    <header>
      <span>{props.icon}</span>
      <div><h3>{props.title}</h3><p>{props.description}</p></div>
      <strong aria-live="polite"><i/>{props.status}</strong>
    </header>
    {props.children}
    <footer><p>{props.hint}</p><div>{props.actions}</div></footer>
  </section>;
}

export function StatusRefreshIcon(props: { spin?: boolean }) {
  return <svg aria-hidden="true" className={props.spin ? "spin" : ""} viewBox="0 0 24 24"><path d="M20 6v5h-5M18 16a7 7 0 1 1 1.5-7.6L20 11"/></svg>;
}
