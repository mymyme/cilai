import { Activity, type ReactNode } from "react";
import { createPortal as nativePortal } from "react-dom";
import { usePageActive } from "./page-visibility";

function PagePortal(props: { children: ReactNode; container: Element | DocumentFragment }) {
  const active = usePageActive();
  return nativePortal(<Activity mode={active ? "visible" : "hidden"}>{props.children}</Activity>,props.container);
}
/** Portals inherit page visibility even though their DOM lives under body. */
export function createPortal(children: ReactNode, container: Element | DocumentFragment, key?: string | null) {
  return <PagePortal key={key} container={container}>{children}</PagePortal>;
}
