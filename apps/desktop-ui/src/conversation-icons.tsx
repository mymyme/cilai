import { type ReactNode } from "react";
import { PanelToggleIcon } from "./icons";


export function Icon({ children }: { children: ReactNode }) { return <svg aria-hidden="true" viewBox="0 0 24 24">{children}</svg>; }

export function PlusIcon() { return <Icon><path d="M12 5v14M5 12h14"/></Icon>; }

export function PinIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M8 3h4l-.6 4.2 2.6 2.3H5.9l2.7-2.3z"/><path d="M10 9.5V16"/></svg>; }

export function MoreDotsIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M5 10h.01M10 10h.01M15 10h.01"/></svg>; }

export function ProjectFolderIcon({ expanded }: { expanded: boolean }) {
  return <svg aria-hidden="true" data-state={expanded ? "expanded" : "collapsed"} viewBox="0 0 20 20">
    {expanded ? <><path d="M2.5 14.8V5.5h5l2 2h8v2"/><path d="M3.8 9.5h13.7l-2 6H2.5l1.3-6Z"/></> : <path d="M2.5 5.5h5l2 2h8v8h-15v-10Z"/>}
  </svg>;
}

export function ProjectGroupIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><rect x="2.5" y="2.5" width="6" height="6" rx="1"/><rect x="11.5" y="2.5" width="6" height="6" rx="1"/><rect x="2.5" y="11.5" width="6" height="6" rx="1"/><rect x="11.5" y="11.5" width="6" height="6" rx="1"/></svg>; }

export function ConversationIcon() { return <Icon><path d="M5 5.5h14v10H9l-4 3v-13Z"/></Icon>; }

export function ContextUsageIcon({ percent }: { percent: number }) {
  const circumference = Math.PI * 17;
  return <svg aria-hidden="true" viewBox="0 0 24 24"><circle className="ai-context-usage-track" cx="12" cy="12" r="8.5"/><circle className="ai-context-usage-value" cx="12" cy="12" r="8.5" strokeDasharray={`${circumference * percent / 100} ${circumference}`}/></svg>;
}

export function ShieldIcon() { return <Icon><path d="M12 3 19 6v5c0 4.5-2.8 8-7 10-4.2-2-7-5.5-7-10V6l7-3Z"/><path d="m9 12 2 2 4-4"/></Icon>; }

export function ApprovalIcon() { return <Icon><path d="M8.5 11V6.5a1.5 1.5 0 0 1 3 0V10 5.5a1.5 1.5 0 0 1 3 0V10 7a1.5 1.5 0 0 1 3 0v6.5A6.5 6.5 0 0 1 11 20h-.5A6.5 6.5 0 0 1 4 13.5V11a1.5 1.5 0 0 1 3 0v2"/></Icon>; }

export function EnterIcon() { return <svg aria-hidden="true" className="ai-enter-icon" viewBox="0 0 24 24"><path d="M19 5v5a5 5 0 0 1-5 5H5"/><path d="m9 11-4 4 4 4"/></svg>; }

export function QueueIcon() { return <Icon><path d="M4.5 7h15"/><path d="M4.5 12h10"/><path d="M4.5 17h6"/></Icon>; }

export function QueueAdjustIcon() { return <Icon><path d="M6.5 5v6.5a3 3 0 0 0 3 3h8"/><path d="m14.5 11 3.5 3.5-3.5 3.5"/></Icon>; }

export function StopIcon() { return <Icon><rect x="7" y="7" width="10" height="10" rx="1.5"/></Icon>; }

export function CopyIcon() { return <Icon><rect x="8" y="8" width="11" height="11" rx="2"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"/></Icon>; }

export function EditIcon() { return <Icon><path d="M5 19h4L19 9l-4-4L5 15v4Z"/><path d="m13.5 6.5 4 4"/></Icon>; }

export function RefreshIcon() { return <Icon><path d="M20 6v5h-5"/><path d="M18 16a7 7 0 1 1 1.5-7.6L20 11"/></Icon>; }

export function CloseIcon() { return <Icon><path d="m7 7 10 10M17 7 7 17"/></Icon>; }

export function ExpandIcon() { return <Icon><path d="M9 4H4v5M15 4h5v5M9 20H4v-5M15 20h5v-5"/></Icon>; }

export function ChevronIcon({ direction }: { direction: "down" | "left" | "right" }) {
  const path = direction === "down" ? "m7 10 5 5 5-5" : direction === "right" ? "m10 7 5 5-5 5" : "m14 7-5 5 5 5";
  return <Icon><path d={path}/></Icon>;
}

export function ConversationRailToggle({ expanded, onToggle }: { expanded: boolean; onToggle: () => void }) {
  const label = expanded ? "收起对话记录" : "展开对话记录";
  return <button aria-controls="ai-conversation-rail" aria-expanded={expanded} aria-label={label} className="ai-conversation-rail-toggle" data-tip={label} onClick={onToggle} type="button"><PanelToggleIcon expanded={expanded}/></button>;
}

export function ArrowDownIcon() { return <Icon><path d="M12 5v13M7 13l5 5 5-5"/></Icon>; }

export function DownIcon() { return <Icon><path d="m8 10 4 4 4-4"/></Icon>; }

export function ReasoningActivityIcon() { return <Icon><path d="M8 16.5h8M9.5 20h5"/><path d="M8.2 13.5A6 6 0 1 1 15.8 13.5c-1 .8-1.5 1.5-1.6 2.2H9.8c-.1-.7-.6-1.4-1.6-2.2Z"/></Icon>; }

export function CommandActivityIcon() { return <Icon><rect x="3.5" y="5" width="17" height="14" rx="2.5"/><path d="m7 10 2.5 2L7 14M12 15h5"/></Icon>; }

export function ToolActivityIcon() { return <Icon><path d="M8.5 4.5 12 3l3.5 1.5L17 8l3.5 1.5V14L17 15.5 15.5 19 12 20.5 8.5 19 7 15.5 3.5 14V9.5L7 8l1.5-3.5Z"/><circle cx="12" cy="12" r="2.5"/></Icon>; }

export function FileChangeActivityIcon() { return <Icon><path d="M6 3h8l4 4v14H6V3Z"/><path d="M14 3v5h5M9 16l5-5 2 2-5 5H9v-2Z"/></Icon>; }

export function PlanActivityIcon() { return <Icon><path d="m5 7 1.5 1.5L9 6M11 7h8M5 12l1.5 1.5L9 11M11 12h8M5 17l1.5 1.5L9 16M11 17h8"/></Icon>; }

export function ContextActivityIcon() { return <Icon><path d="M4 5h16M4 19h16M8 9l4 3 4-3M8 15l4-3 4 3"/></Icon>; }

export function SearchBusinessIcon() { return <Icon><circle cx="10.5" cy="10.5" r="5.5"/><path d="m15 15 5 5"/></Icon>; }

export function WriteBusinessIcon() { return <Icon><path d="M5 19h4L19 9l-4-4L5 15v4Z"/><path d="m13.5 6.5 4 4M5 19l3-3"/></Icon>; }

export function FillBusinessIcon() { return <Icon><rect x="5" y="3" width="14" height="18" rx="2"/><path d="M9 3.5h6V7H9zM9 11h6M9 15h6"/></Icon>; }

export function AdvanceBusinessIcon() { return <Icon><path d="M4 18V6M4 18h16"/><path d="m7 14 4-4 3 2 5-6"/><path d="M16 6h3v3"/></Icon>; }

export function ArrowRightIcon() { return <Icon><path d="M6 12h12M14 8l4 4-4 4"/></Icon>; }

export function FileIcon() { return <Icon><path d="M6 3h8l4 4v14H6V3Z"/><path d="M14 3v5h5M9 12h6M9 16h6"/></Icon>; }

export function ImageMaterialIcon() { return <Icon><rect x="4" y="4" width="16" height="16" rx="2.5"/><circle cx="9" cy="9" r="1.5"/><path d="m6.5 17 4-4 2.5 2.5 2-2 2.5 3.5"/></Icon>; }

export function OutputIcon() { return <Icon><rect x="3" y="4" width="18" height="16" rx="3"/><path d="M7 16v-4M11 16V9M15 16v-6M19 16V7"/></Icon>; }

export function ContextAddIcon() { return <Icon><path d="M3.5 6.5h6l2 2h9v11h-17v-13Z"/><path d="M15.5 11.5v5M13 14h5"/></Icon>; }

export function CheckIcon() { return <Icon><path d="m6 12 4 4 8-8"/></Icon>; }

export function PaperclipIcon() { return <Icon><path d="m9 12.5 5.8-5.8a3 3 0 0 1 4.2 4.2l-7.3 7.3a5 5 0 0 1-7.1-7.1l7-7"/><path d="m7.5 14.5 7.4-7.4"/></Icon>; }

export function MediaIcon() { return <Icon><rect x="3.5" y="5" width="17" height="14" rx="2.5"/><circle cx="9" cy="10" r="1.5"/><path d="m5.5 17 4-4 3 3 2.5-2.5 3.5 3.5"/><path d="m16 7.5 3 2-3 2v-4Z"/></Icon>; }

export function SkillIcon() { return <Icon><path d="M9 3h6l1 3 3 1v6l-3 1-1 3H9l-1-3-3-1V7l3-1 1-3Z"/><circle cx="12" cy="10" r="2.5"/></Icon>; }

export function ReuseIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M4 7a6 6 0 0 1 10-2l2 2M16 3v4h-4M16 13a6 6 0 0 1-10 2l-2-2M4 17v-4h4"/></svg>; }

export function ArtifactsSourcesIcon({ open }: { open: boolean }) {
  return <svg aria-hidden="true" data-open={open} viewBox="0 0 20 20">
    {open ? <path className="ai-info-panel-fill" d="M5 2.5h6.5L16 7v9.5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1v-13a1 1 0 0 1 1-1Z"/> : null}
    <path d="M11.5 2.5H5a1 1 0 0 0-1 1v13a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1V7Zm0 0V7H16M7 10h6M7 13h4"/>
  </svg>;
}
