import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { rpc } from "./app-server-rpc";
import type { WorkspaceDirectoryEntry, WorkspaceFileItem, WorkspaceListDirectoryResult } from "./generated/protocol";
import { fileDisplayStatus } from "./file-status";
import { ChevronIcon, FileIcon, TreeFolderIcon } from "./icons";

const ROW_HEIGHT = 32;
const OVERSCAN = 10;

/** 一层目录的读取状态。 */
type Level =
  | { state: "loading" }
  | { state: "ready"; entries: WorkspaceDirectoryEntry[]; issueCount: number }
  | { state: "error"; message: string };

type Row =
  | { kind: "entry"; entry: WorkspaceDirectoryEntry; depth: number }
  | { kind: "status"; key: string; text: string; tone: "muted" | "danger"; depth: number; retryPath?: string };

const ROOT = "";

export function isHiddenEntry(path: string) {
  return path.split("/").some((segment) => segment.startsWith(".") && segment !== "." && segment !== "..");
}

/**
 * 把已读到的各层目录按展开状态摊平成可见行。未展开的目录不产出子行，
 * 正在读或读失败的层以一行状态代替子项。
 */
export function flattenLevels(levels: Map<string, Level>, expanded: Set<string>, showHidden: boolean): Row[] {
  const rows: Row[] = [];
  const walk = (path: string, depth: number) => {
    const level = levels.get(path);
    if (!level || level.state === "loading") {
      rows.push({ kind: "status", key: `loading:${path}`, text: "正在读取…", tone: "muted", depth });
      return;
    }
    if (level.state === "error") {
      rows.push({ kind: "status", key: `error:${path}`, text: level.message, tone: "danger", depth, retryPath: path });
      return;
    }
    const entries = showHidden ? level.entries : level.entries.filter((entry) => !isHiddenEntry(entry.relativePath));
    for (const entry of entries) {
      rows.push({ kind: "entry", entry, depth });
      if (entry.fileType === "directory" && expanded.has(entry.relativePath)) walk(entry.relativePath, depth + 1);
    }
    if (level.issueCount > 0) rows.push({ kind: "status", key: `issues:${path}`, text: `有 ${level.issueCount} 项无法读取`, tone: "danger", depth });
    if (entries.length === 0 && level.issueCount === 0) rows.push({ kind: "status", key: `empty:${path}`, text: "空文件夹", tone: "muted", depth });
  };
  walk(ROOT, 0);
  return rows;
}

function describeListError(error: unknown) {
  const text = String(error);
  if (text.includes("workspace_directory_busy")) return "这个目录仍在读取，请稍后重试";
  if (text.includes("workspace_directory_capacity_reached")) return "同时读取的目录较多，请稍后重试";
  if (text.includes("workspace_target_unavailable")) return "这个文件夹已不存在或无法读取";
  if (text.includes("workspace_root_unavailable")) return "工作区文件夹无法访问";
  return `读取失败：${text}`;
}

/**
 * 本地文件树直接读取磁盘目录项，索引仅提供文件引用和内容状态。
 *
 * 读目录只看磁盘、不读文件内容，所以很大的文件夹也能马上进去看。已经进了索引的
 * 文件显示状态、可直接预览；还没进索引的文件点击后先单独补录它，再打开。
 */
export function LazyFileTree(props: {
  workspaceId: string;
  workspaceKind: string;
  indexedFiles: WorkspaceFileItem[];
  showHidden: boolean;
  selectedFileId: string | null;
  selectedDirectoryPath?: string | null;
  pendingPath: string | null;
  /** 变化时重读所有已读过的层，用于新建、删除之后刷新；展开状态保留。 */
  refreshToken?: number;
  onDirectory?: (path: string) => void;
  onFile: (id: string) => void;
  onUnindexedFile: (relativePath: string) => void;
  onContext: (event: React.MouseEvent, file: WorkspaceFileItem) => void;
}) {
  const { workspaceId } = props;
  const [levels, setLevels] = useState<Map<string, Level>>(() => new Map());
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);
  const container = useRef<HTMLDivElement>(null);
  const generation = useRef(0);
  const requests = useRef(new Map<string, number>());

  const loadLevel = useCallback(async (path: string, quiet = false) => {
    const current = generation.current;
    const request = (requests.current.get(path) ?? 0) + 1;
    requests.current.set(path, request);
    if (!quiet) setLevels((previous) => new Map(previous).set(path, { state: "loading" }));
    try {
      const result = await rpc<WorkspaceListDirectoryResult>("workspace/listDirectory", { workspaceId, relativePath: path || null });
      if (current !== generation.current || requests.current.get(path) !== request) return;
      setLevels((previous) => {
        const level = previous.get(path);
        if (level?.state === "ready" && level.issueCount === result.issueCount && JSON.stringify(level.entries) === JSON.stringify(result.entries)) return previous;
        return new Map(previous).set(path, { state: "ready", entries: result.entries, issueCount: result.issueCount });
      });
    } catch (error) {
      if (current !== generation.current || requests.current.get(path) !== request) return;
      setLevels((previous) => new Map(previous).set(path, { state: "error", message: describeListError(error) }));
    }
  }, [workspaceId]);

  useEffect(() => {
    generation.current += 1;
    setLevels(new Map());
    setExpanded(new Set());
    void loadLevel(ROOT);
    return () => { generation.current += 1; };
  }, [loadLevel]);

  const selectedPath = props.selectedDirectoryPath ?? props.indexedFiles.find((file) => file.localFileId === props.selectedFileId)?.relativePath;
  useEffect(() => {
    if (!selectedPath) return;
    const segments = selectedPath.replaceAll("\\", "/").split("/");
    const parents = segments.slice(0, -1).map((_, index) => segments.slice(0, index + 1).join("/"));
    if (parents.length === 0) return;
    setExpanded((current) => new Set([...current, ...parents]));
    for (const path of parents) void loadLevel(path, true);
  }, [selectedPath, loadLevel]);

  // 只跟着 refreshToken 走：换工作区时由上面的重置负责，不能拿旧工作区读过的路径去读新工作区。
  const loadedPaths = useRef<string[]>([]);
  loadedPaths.current = [ROOT];
  const collectExpanded = (parent: string) => {
    const level = levels.get(parent);
    if (level?.state !== "ready") return;
    for (const entry of level.entries) {
      if (entry.fileType === "directory" && expanded.has(entry.relativePath) && (props.showHidden || !isHiddenEntry(entry.relativePath))) {
        loadedPaths.current.push(entry.relativePath);
        collectExpanded(entry.relativePath);
      }
    }
  };
  collectExpanded(ROOT);
  const refreshablePaths = useRef<string[]>([]);
  refreshablePaths.current = loadedPaths.current.filter((path) => levels.get(path)?.state === "ready");
  const loadLevelRef = useRef(loadLevel);
  loadLevelRef.current = loadLevel;
  const firstRefresh = useRef(true);
  useEffect(() => {
    if (firstRefresh.current) { firstRefresh.current = false; return; }
    for (const path of loadedPaths.current) void loadLevelRef.current(path, true);
  }, [props.refreshToken]);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const refresh = async () => {
      if (document.visibilityState !== "hidden") {
        await Promise.all(refreshablePaths.current.map((path) => loadLevelRef.current(path, true)));
      }
      if (!cancelled) timer = setTimeout(() => void refresh(), 3000);
    };
    timer = setTimeout(() => void refresh(), 3000);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [workspaceId]);

  useEffect(() => {
    const element = container.current;
    if (!element) return;
    const resize = new ResizeObserver(() => setViewportHeight(element.clientHeight));
    resize.observe(element);
    setViewportHeight(element.clientHeight);
    return () => resize.disconnect();
  }, []);

  const indexByPath = useMemo(() => new Map(props.indexedFiles.filter((file) => file.presenceState === "present").map((file) => [file.relativePath.replaceAll("\\", "/"), file])), [props.indexedFiles]);
  const rows = useMemo(() => flattenLevels(levels, expanded, props.showHidden), [levels, expanded, props.showHidden]);

  const toggleDirectory = (path: string) => {
    const opening = !expanded.has(path);
    setExpanded((current) => {
      const next = new Set(current);
      if (opening) next.add(path); else next.delete(path);
      return next;
    });
    if (opening) void loadLevel(path, levels.get(path)?.state === "ready");
  };

  const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const visibleCount = Math.ceil(viewportHeight / ROW_HEIGHT) + OVERSCAN * 2;
  const end = Math.min(rows.length, start + Math.max(visibleCount, 30));

  return <div aria-label="文件列表（逐层浏览）" className="file-tree" onClick={(event) => { if (!(event.target as HTMLElement).closest("button")) props.onDirectory?.(""); }} onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)} ref={container}>
    <div className="file-tree-spacer" style={{ height: rows.length * ROW_HEIGHT }}>
      <div className="file-tree-items" style={{ transform: `translateY(${start * ROW_HEIGHT}px)` }}>
        {rows.slice(start, end).map((row) => {
          if (row.kind === "status") {
            return <div className="file-tree-status" data-tone={row.tone} key={row.key} style={{ paddingLeft: 23 + row.depth * 14 }}>
              <span>{row.text}</span>
              {row.retryPath !== undefined ? <button onClick={() => void loadLevel(row.retryPath ?? ROOT)} type="button">重试</button> : null}
            </div>;
          }
          const { entry, depth } = row;
          const directory = entry.fileType === "directory";
          const indexed = indexByPath.get(entry.relativePath);
          const status = !directory && indexed ? fileDisplayStatus(indexed.displayState, props.workspaceKind) : null;
          const selected = directory ? entry.relativePath === props.selectedDirectoryPath : indexed !== undefined && indexed.localFileId === props.selectedFileId;
          const pending = props.pendingPath === entry.relativePath;
          const disabled = !directory && entry.fileType !== "file";
          const isOpen = directory && expanded.has(entry.relativePath);
          return <button
            aria-busy={pending || undefined}
            aria-expanded={directory ? isOpen : undefined}
            className={`${selected ? "selected" : ""} ${directory ? "directory" : ""}`}
            disabled={disabled || pending}
            key={entry.relativePath}
            onClick={() => {
              if (directory) { props.onDirectory?.(entry.relativePath); toggleDirectory(entry.relativePath); }
              else if (indexed) props.onFile(indexed.localFileId);
              else props.onUnindexedFile(entry.relativePath);
            }}
            onContextMenu={(event) => { if (indexed) props.onContext(event, indexed); }}
            style={{ paddingLeft: 6 + depth * 14 }}
            title={pending ? `${entry.relativePath}（正在建立索引）` : entry.relativePath}
            type="button">
            {directory ? <><ChevronIcon collapsed={!isOpen}/><TreeFolderIcon/></> : <><span className="tree-indent"/><FileIcon/></>}
            <span>{entry.name}</span>
            {status ? <i aria-hidden="true" data-file-status={status.state} data-tone={status.tone} title={status.label}/> : <i className="file-state-spacer"/>}
          </button>;
        })}
      </div>
    </div>
  </div>;
}
