/**
 * 界面里的线性图标。它们此前和应用主体挤在同一个文件里，占掉三十来行却和任何逻辑无关，
 * 读主流程时得反复跳过。stroke 相关的样式统一由各处的 svg 选择器给出，这里只出路径。
 */
export function Folder() { return <svg aria-hidden="true" viewBox="0 0 48 48"><path d="M7.5 13.5h13l4 4h16v21h-33v-25Z"/><path d="M13.5 25h21M13.5 31h14"/></svg>; }
export function FileIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M5 2.5h6l4 4v11H5z"/><path d="M11 2.5v4h4"/></svg>; }
export function TreeFolderIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M2.5 5.5h6l1.8 2H17v8.5H2.5z"/></svg>; }
export function ChevronIcon({ collapsed }: { collapsed: boolean }) { return <svg aria-hidden="true" className="tree-chevron" viewBox="0 0 16 16"><path d={collapsed ? "m6 4 4 4-4 4" : "m4 6 4 4 4-4"}/></svg>; }
export function WorkspaceIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M4 4h16v16H4z"/><path d="M8 8h8M8 12h8M8 16h5"/></svg>; }
export function HomeIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="m3 10 9-7 9 7"/><path d="M5 8.5V21h5v-7h4v7h5V8.5"/></svg>; }
export function CaptureIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><rect x="8" y="3" width="8" height="12" rx="4"/><path d="M5 10v2a7 7 0 0 0 14 0v-2M12 19v3M8 22h8"/></svg>; }
export function ChatBubbleIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M21 15a3 3 0 0 1-3 3H8l-5 3V6a3 3 0 0 1 3-3h12a3 3 0 0 1 3 3Z"/></svg>; }
export function PortfolioIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M4 5h7v6H4zM13 5h7v10h-7zM4 13h7v6H4zM13 17h7v2h-7z"/></svg>; }
export function GlobalSearchIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="10.5" cy="10.5" r="6.5"/><path d="m15.5 15.5 5 5"/></svg>; }
export function AutomationIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="12" cy="12" r="4"/><path d="M12 3v5M12 16v5M3 12h5M16 12h5M5.6 5.6l3.5 3.5M14.9 14.9l3.5 3.5M18.4 5.6l-3.5 3.5M9.1 14.9l-3.5 3.5"/></svg>; }
export function ApplicationsIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><rect x="4" y="4" width="6" height="6" rx="1.5"/><rect x="14" y="4" width="6" height="6" rx="1.5"/><rect x="4" y="14" width="6" height="6" rx="1.5"/><path d="M17 14v6M14 17h6"/></svg>; }
export function SkillIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="m12 3 7 4-7 4-7-4 7-4Z"/><path d="m5 12 7 4 7-4M5 17l7 4 7-4"/></svg>; }
export function PlusIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M10 3v14M3 10h14"/></svg>; }
export function PencilIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="m12.8 3.2 4 4L7 17H3v-4L12.8 3.2Z"/><path d="m10.5 5.5 4 4"/></svg>; }
export function TrashIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M4 6h12M8 3h4l1 3H7l1-3ZM6 6l1 11h6l1-11M9 9v5M11 9v5"/></svg>; }
export function SparkIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="m10 2 1.4 4.6L16 8l-4.6 1.4L10 14l-1.4-4.6L4 8l4.6-1.4L10 2Z"/><path d="m16 13 .6 1.9 1.9.6-1.9.6L16 19l-.6-1.9-1.9-.6 1.9-.6L16 13Z"/></svg>; }
export function PanelToggleIcon({ expanded }: { expanded: boolean }) {
  return <svg aria-hidden="true" viewBox="0 0 24 24"><rect x="4" y="6" width="16" height="12" rx="2"/><path d={expanded ? "M9 8.5v7" : "M7 8.5v7"}/></svg>;
}
export function AiIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="m12 3 1.7 5.3L19 10l-5.3 1.7L12 17l-1.7-5.3L5 10l5.3-1.7L12 3Z"/><path d="m19 16 .7 2.3L22 19l-2.3.7L19 22l-.7-2.3L16 19l2.3-.7L19 16Z"/></svg>; }
export function AgentIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><rect x="5" y="7" width="14" height="12" rx="3"/><path d="M12 3v4M8 12h.01M16 12h.01M9 16h6"/></svg>; }
export function RunsIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M5 12h3l2-5 4 10 2-5h3"/><path d="M4 4h16v16H4z"/></svg>; }
export function UserIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="12" cy="8" r="3.5"/><path d="M5.5 20v-2.2A6.5 6.5 0 0 1 12 11.5a6.5 6.5 0 0 1 6.5 6.3V20"/></svg>; }
export function SunIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="12" cy="12" r="3.5"/><path d="M12 2.5v3M12 18.5v3M2.5 12h3M18.5 12h3M5.3 5.3l2.1 2.1M16.6 16.6l2.1 2.1M18.7 5.3l-2.1 2.1M7.4 16.6l-2.1 2.1"/></svg>; }
export function MoonIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M20 15.4A8.5 8.5 0 0 1 8.6 4a8.5 8.5 0 1 0 11.4 11.4Z"/></svg>; }
export function ArchiveIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M4 7h16v13H4zM3 3h18v4H3zM9 11h6"/></svg>; }
export function SettingsIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1-2.8 2.8-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.6v.2h-4V21a1.7 1.7 0 0 0-1-1.6 1.7 1.7 0 0 0-1.9.3l-.1.1L4.2 17l.1-.1a1.7 1.7 0 0 0 .3-1.9A1.7 1.7 0 0 0 3 14H2.8v-4H3a1.7 1.7 0 0 0 1.6-1 1.7 1.7 0 0 0-.3-1.9L4.2 7 7 4.2l.1.1A1.7 1.7 0 0 0 9 4.6a1.7 1.7 0 0 0 1-1.6v-.2h4V3a1.7 1.7 0 0 0 1 1.6 1.7 1.7 0 0 0 1.9-.3l.1-.1L19.8 7l-.1.1a1.7 1.7 0 0 0-.3 1.9 1.7 1.7 0 0 0 1.6 1h.2v4H21a1.7 1.7 0 0 0-1.6 1Z"/></svg>; }
export function RefreshIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M20 11a8 8 0 0 0-14.7-4.4L3 10"/><path d="M3 4v6h6M4 13a8 8 0 0 0 14.7 4.4L21 14"/><path d="M21 20v-6h-6"/></svg>; }
export function EyeIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M2.5 12s3.5-6 9.5-6 9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6Z"/><circle cx="12" cy="12" r="2.5"/></svg>; }
export function EyeOffIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="m3 3 18 18M10.6 6.1A10.5 10.5 0 0 1 12 6c6 0 9.5 6 9.5 6a16.8 16.8 0 0 1-2.1 2.8M6.1 6.1A17.2 17.2 0 0 0 2.5 12s3.5 6 9.5 6a9.8 9.8 0 0 0 3.2-.5M9.9 9.9a3 3 0 0 0 4.2 4.2"/></svg>; }
export function CodeIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="m8 7-5 5 5 5M16 7l5 5-5 5M14 4l-4 16"/></svg>; }
export function AddFolderIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M3 6h7l2 2h9v11H3z"/><path d="M12 11v5M9.5 13.5h5"/></svg>; }
export function SwitcherIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="m6 7 4-4 4 4M14 13l-4 4-4-4"/></svg>; }
export function SearchIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="10.5" cy="10.5" r="6.5"/><path d="m15.5 15.5 5 5"/></svg>; }
export function CheckIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="m4 10 4 4 8-9"/></svg>; }
export function RevealIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M3 6h7l2 2h9v11H3z"/><path d="m13 11 4 4M17 11v4h-4"/></svg>; }
export function BackIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="m11.5 5-5 5 5 5M7 10h7"/></svg>; }
export function DiffIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M4 3v14M16 3v14M7 7h6M10 4l3 3-3 3M13 13H7M10 10l-3 3 3 3"/></svg>; }
export function InfoIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><circle cx="10" cy="10" r="7"/><path d="M10 9v5M10 6h.01"/></svg>; }
export function ExternalOpenIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M11 4h5v5M16 4l-7 7"/><path d="M14 11v5H4V6h5"/></svg>; }
export function ConfirmIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M10 2.5 3 5.5v4.2c0 3.4 2.8 6.2 7 7.8 4.2-1.6 7-4.4 7-7.8V5.5Z"/><path d="m7.2 10 2 2 3.6-3.8"/></svg>; }
export function BackArrowIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="m11 5-5 5 5 5M6.5 10H15"/></svg>; }
export function MoreIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M4.8 10h.01M10 10h.01M15.2 10h.01"/></svg>; }
export function WindowMinimizeIcon() { return <svg aria-hidden="true" viewBox="0 0 16 16"><path d="M2.5 8h11"/></svg>; }
export function WindowMaximizeIcon() { return <svg aria-hidden="true" viewBox="0 0 16 16"><rect x="2.5" y="2.5" width="11" height="11" rx=".5"/></svg>; }
export function WindowRestoreIcon() { return <svg aria-hidden="true" viewBox="0 0 16 16"><path d="M5.5 5.5v-3h8v8h-3"/><rect x="2.5" y="5.5" width="8" height="8" rx=".5"/></svg>; }
export function WindowCloseIcon() { return <svg aria-hidden="true" viewBox="0 0 16 16"><path d="m3 3 10 10M13 3 3 13"/></svg>; }
export function CloseIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="m5 5 10 10M15 5 5 15"/></svg>; }
