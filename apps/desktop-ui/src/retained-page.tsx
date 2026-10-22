import { Suspense, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { PageVisibilityContext, usePageActive } from "./page-visibility";
import "./retained-page.css";

/** Retain state, DOM and preview resources until the account scope closes.
 * Visibility is explicit so menus, focus handlers and display polling can pause.
 */
export function RetainedPage(props: { active: boolean; children: ReactNode; settings?: boolean }) {
  const parentActive = usePageActive();
  const active = props.active && parentActive;
  const [visited, setVisited] = useState(active);
  const root = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (!active) root.current?.querySelectorAll<HTMLMediaElement>("audio,video").forEach(media => media.pause());
  },[active]);
  if (active && !visited) setVisited(true);
  if (!active && !visited) return null;
  return <PageVisibilityContext.Provider value={active}>
    <div ref={root} hidden={!active} inert={!active} className="retained-page">
    <Suspense fallback={<div className={props.settings ? "settings-loading" : "page-loading"} role="status">正在加载…</div>}>
      {props.children}
    </Suspense>
    </div>
  </PageVisibilityContext.Provider>;
}
