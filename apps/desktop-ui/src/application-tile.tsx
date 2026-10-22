import type { ReactNode } from "react";
import { ApplicationBusinessIcon, applicationIconKind } from "./application-icons";
import { MoreIcon } from "./icons";
import { RichMenu } from "./rich-menu";

/**
 * 应用页宫格条目：企业自建与我的创建沿用平台应用的图标宫格外观。
 * 点击图标执行主操作；其余操作收进右上角的更多菜单，菜单按钮与主按钮并列而非嵌套。
 */
export function ApplicationTile(props: {
  name: string;
  description?: string;
  status: string;
  statusTone?: string;
  actionLabel: string;
  busy: boolean;
  onOpen: () => void;
  menu?: ReactNode;
}) {
  const iconKind = applicationIconKind({
    name: props.name,
    keywords: [],
    description: props.description,
    categoryLabel: "",
  });
  return (
    <article className="application-tile" aria-label={props.name}>
      <button
        aria-busy={props.busy || undefined}
        aria-label={`${props.name}，${props.actionLabel}`}
        className="application-card"
        disabled={props.busy}
        onClick={props.onOpen}
        title={[props.name, props.description].filter(Boolean).join("：")}
        type="button"
      >
        <span className="application-mark" data-tone={iconKind} aria-hidden="true">
          <ApplicationBusinessIcon kind={iconKind} />
        </span>
        <strong className="application-name">{props.name}</strong>
        <small className="application-status" data-status={props.statusTone}>{props.status}</small>
      </button>
      {props.menu ? (
        <RichMenu ariaLabel={`${props.name}的更多操作`} align="end" width={192} keyboardNavigation
          trigger={({ open, toggle, triggerRef }) => (
            <button className="application-tile-more" aria-label={`${props.name}的更多操作`} aria-haspopup="menu" aria-expanded={open} data-open={open || undefined} disabled={props.busy} onClick={toggle} ref={triggerRef} title="更多" type="button"><MoreIcon /></button>
          )}>
          {props.menu}
        </RichMenu>
      ) : null}
    </article>
  );
}
