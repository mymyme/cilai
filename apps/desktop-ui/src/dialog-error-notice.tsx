import { PageNoticeStack, type PageError } from "@cilai/app-ui/page-notice";
import { useId, useLayoutEffect, useState } from "react";
import { usePageActive } from "./page-visibility";

export type DialogError = { message: string; tone?: PageError["tone"]; actions?: PageError["actions"] };

export function useDialogErrorNotice() {
  const noticeLayer = `dialog-notice-${useId()}`;
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const [content, setContent] = useState<HTMLElement | null>(null);
  return { noticeLayer, anchor, setAnchor, content, setContent };
}

export function DialogErrorNotice({ layer, error }: {
  layer: ReturnType<typeof useDialogErrorNotice>; error?: DialogError | readonly DialogError[] | null;
}) {
  const active = usePageActive();
  const [dismissed, setDismissed] = useState<readonly string[]>([]);
  const errors = error && "message" in error ? [error] : error ?? [];
  const notices = errors.flatMap((item, index): PageError[] => item.message ? [{
    ...item, kind: "error", tone: item.tone ?? "danger", id: JSON.stringify([index, item.tone ?? "danger", item.message]),
  }] : []);
  useLayoutEffect(() => {
    setDismissed((current) => {
      const live = current.filter((id) => notices.some((notice) => notice.id === id));
      return live.length === current.length ? current : live;
    });
  });
  return <PageNoticeStack active={active} anchor={layer.anchor} content={layer.content}
    className={`workbench-page-notice workbench-dialog-notice ${layer.noticeLayer}`}
    notices={notices.filter((notice) => !dismissed.includes(notice.id))}
    onDismiss={(id) => setDismissed((current) => [...current, id])}/>;
}
