import type { ButtonHTMLAttributes, ComponentProps, FormEventHandler, ReactNode, Ref } from "react";
import { MarkdownContent } from "./markdown-content";
import { RichMenu, RichMenuItem, RichMenuSection } from "./rich-menu";
import "./ai-workbench.css";

export function ConversationStage({ children, panel, tools, docked, panelOpen, stageRef }: {
  children: ReactNode;
  panel?: ReactNode;
  tools?: ReactNode;
  docked?: boolean;
  panelOpen?: boolean;
  stageRef?: Ref<HTMLElement>;
}) {
  return <main className="ai-chat-stage" ref={stageRef}>
    <div className={`ai-conversation-shell${panelOpen ? " info-open" : ""}${panelOpen && docked ? " info-docked" : " info-popover"}`}>
      <div className="ai-conversation-main">{children}</div>
      {tools}
      {panel}
    </div>
  </main>;
}

export function ConversationMessageList({ children, landing, navigator, after, listProps }: {
  children: ReactNode;
  landing: boolean;
  navigator?: ReactNode;
  after?: ReactNode;
  listProps?: Omit<ComponentProps<"div">, "children" | "className">;
}) {
  return <div className="ai-message-scroll-shell">
    {navigator}
    <div {...listProps} className={landing ? "ai-message-list ai-message-list-landing" : "ai-message-list"}>{children}</div>
    {after}
  </div>;
}

export function ConversationLatestButton({ unseen, onClick }: { unseen?: boolean; onClick: () => void }) {
  return <button aria-label={unseen ? "回到底部，有新输出" : "回到底部"} className="ai-return-to-latest" onClick={onClick} title="回到底部" type="button">
    <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M12 5v13M7 13l5 5 5-5"/></svg>
    {unseen ? <i aria-hidden="true"/> : null}
  </button>;
}

export function ConversationMessage({ children, role, author, meta, messageId, className, pending }: {
  children: ReactNode;
  role: "user" | "assistant";
  author: string;
  meta?: ReactNode;
  messageId?: string;
  className?: string;
  pending?: boolean;
}) {
  return <article
    aria-label={pending ? undefined : role === "user" ? "用户消息" : "AI 消息"}
    className={`ai-message ${role}${className ? ` ${className}` : ""}`}
    data-message-id={messageId}
    role={pending ? "status" : undefined}>
    <header><span>{author}</span>{meta}</header>
    {children}
  </article>;
}

export function ConversationMessageBody({ role, content, markdown, className, live }: {
  role: "user" | "assistant";
  content: string;
  markdown?: Omit<ComponentProps<typeof MarkdownContent>, "children">;
  className?: string;
  live?: boolean;
}) {
  return <div className={className} aria-live={live ? "polite" : undefined}>{role === "assistant" ? <MarkdownContent {...markdown}>{content}</MarkdownContent> : content}</div>;
}

export function ConversationComposer({ children, leadingAction, actions, afterActions, onSubmit }: {
  children: ReactNode;
  leadingAction: ReactNode;
  actions: ReactNode;
  afterActions?: ReactNode;
  onSubmit: FormEventHandler<HTMLFormElement>;
}) {
  return <form className="ai-composer" onSubmit={onSubmit}>
    {children}
    <footer>{leadingAction}<div className="ai-composer-actions">{actions}</div></footer>
    {afterActions}
  </form>;
}

export function ConversationActionButton({ children, stop, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { stop?: boolean }) {
  return <button {...props} className={stop ? "ai-send-button ai-stop-button" : "ai-send-button"}>{children ?? (stop
    ? <svg aria-hidden="true" viewBox="0 0 24 24"><rect x="7" y="7" width="10" height="10" rx="1.5"/></svg>
    : <svg aria-hidden="true" className="ai-enter-icon" viewBox="0 0 24 24"><path d="M19 5v5a5 5 0 0 1-5 5H5"/><path d="m9 11-4 4 4 4"/></svg>)}</button>;
}

export function ConversationIconButton(props: ComponentProps<"button">) {
  return <button {...props} className="ai-conversation-icon-button" type="button"/>;
}

export function ConversationInfoRow({ className, ...props }: ComponentProps<"button">) {
  return <button {...props} className={className ? `ai-conversation-info-row ${className}` : "ai-conversation-info-row"} type="button"/>;
}

export function ConversationComposerAdd(props: {
  canAttachMedia: boolean;
  mediaDescription: string;
  disabled: boolean;
  workspaceFileDisabled?: boolean;
  hasFile: boolean;
  hasWorkspaceFiles?: boolean;
  onOpenFiles?: () => void;
  onOpenMedia: () => void;
}) {

  return <RichMenu
    align="start"
    ariaLabel="添加上下文"
    className="ai-add-menu"
    maxHeight={420}
    width={380}
    trigger={({ open, toggle, triggerRef }) => <button
      aria-expanded={open}
      aria-haspopup="menu"
      aria-label={props.onOpenFiles ? "添加上下文" : "添加附件"}
      aria-pressed={props.hasFile}
      className={`ai-context-add${props.hasFile ? " selected" : ""}`}
      disabled={props.disabled}
      onClick={toggle}
      ref={triggerRef}
      title={props.onOpenFiles ? "添加附件或工作区文件" : "添加附件"}
      type="button"><PlusIcon/></button>}>
    <RichMenuSection label="添加">
      <RichMenuItem
        disabled={!props.canAttachMedia}
        description={props.mediaDescription}
        label="附件"
        leading={<MediaIcon/>}
        onSelect={props.onOpenMedia}/>
      {props.onOpenFiles ? <RichMenuItem
        disabled={props.workspaceFileDisabled || !props.hasWorkspaceFiles}
        description={props.workspaceFileDisabled ? "当前回复结束后可以更改工作区文件" : props.hasWorkspaceFiles ? "从当前工作区选择文件" : "当前工作区没有可用文件"}
        label="工作区文件"
        leading={<PaperclipIcon/>}
        onSelect={props.onOpenFiles}/> : null}
    </RichMenuSection>
  </RichMenu>;
}

function Icon({ children }: { children: ReactNode }) { return <svg aria-hidden="true" viewBox="0 0 24 24">{children}</svg>; }
export function PlusIcon() { return <Icon><path d="M12 5v14M5 12h14"/></Icon>; }
export function FileIcon() { return <Icon><path d="M6 3h8l4 4v14H6V3Z"/><path d="M14 3v5h5M9 12h6M9 16h6"/></Icon>; }
export function MediaIcon() { return <Icon><rect x="3.5" y="5" width="17" height="14" rx="2.5"/><circle cx="9" cy="10" r="1.5"/><path d="m5.5 17 4-4 3 3 2.5-2.5 3.5 3.5"/><path d="m16 7.5 3 2-3 2v-4Z"/></Icon>; }
export function CloseIcon() { return <Icon><path d="m7 7 10 10M17 7 7 17"/></Icon>; }
export function PaperclipIcon() { return <Icon><path d="m9 12.5 5.8-5.8a3 3 0 0 1 4.2 4.2l-7.3 7.3a5 5 0 0 1-7.1-7.1l7-7"/><path d="m7.5 14.5 7.4-7.4"/></Icon>; }
