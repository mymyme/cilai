import { ModelCatalogProvider } from "./use-model-catalog";
import { WindowsSandboxProvider } from "./use-windows-sandbox";
import { RetainedPage } from "./retained-page";
import type { SettingsSection } from "./settings";
import { LoadFailure } from "./load-failure";
import { ExportDiagnosticsButton } from "./test-diagnostics";
import { StartupPermissions } from "./startup-permissions";
import { PersonalMaintenanceRecovery, PersonalRestoreStatus } from "./personal-data";
import type { WorkReuseIntent } from "./work-reuse";
import type { RecentConversation } from "./my-work";
import { ApplicationHandoffs } from "./application-handoffs";
import { ConversationSessionProvider } from "./conversation-session";
import { indexWorkspaceProjects, unambiguousWorkspaceProjects, type WorkspaceProjectIdentity } from "./project-workspaces";
import { ArchiveNotice, type ArchiveNoticeOperation } from "./archive-notice";
import { conversationArchiveChanged, invalidateArchiveOperations, type ConversationArchiveChange } from "./conversation-archive";
import { invoke, listen } from "./host";
import { personalNotificationOpenEvent, personalNotificationTarget, type PersonalNotificationOpen } from "./personal-notification-navigation";
import { randomUuid } from "./uuid";
import { HoverTip } from "./hover-tip";
import { StrictMode, Suspense, lazy, useCallback, useEffect, useMemo, useReducer, useRef, useState, type CSSProperties, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import { createRoot } from "react-dom/client";
import type { DeliverableItem, DeliverableListResult, MyWorkItem, ProjectAttachWorkspaceResult, ProjectDetachWorkspaceResult, ProjectGetResult, ProjectPortfolioListResult, UnifiedSearchItem, WorkspaceCreateResult, WorkspaceFileItem, WorkspaceFilesResult, WorkspaceListResult, WorkspaceSummary } from "./generated/protocol";
import type { AiLaunchRequest, AiProjectOption } from "./ai-workbench";
import { workspaceContext, unclassifiedConversationContext } from "./ai-workbench-context";
import { ProjectCreatePanel } from "./project-create";
import { AlertDialog, ConfirmDialog } from "./confirm-dialog";
import { BrowserApprovalSurface } from "./browser-approval";
import { AiBrowserPanel, aiBrowserToggleIntent, useAiBrowserState, useViewportWidth } from "./ai-browser";
import { activityBarWidthFor, clampAiBrowserWidth, aiBrowserWidthStorageKey,
  desiredViewportForAiBrowser, fitAiBrowser, minChatStageWidth, minViewportForAiBrowser,
  readAiBrowserWidth } from "./ai-layout";
// 应用页的账户菜单曾经必须用原生浮层：旧宿主把应用放在子视图里，它永远压在界面之上，
// 普通弹窗会被遮住。新宿主把应用画进界面的布局流，弹窗不再被压住，浮层那条路也就不需要
// 了——留着反而会去调用新宿主并不提供的命令，菜单在应用页上打不开。
import type { AppMessage, MessageSink, MessageTone } from "./app-message";
import { ContextMenu, openContextMenu, type ContextMenuState } from "./context-menu";
import { ProjectEditDialog, type ProjectEditTarget } from "./project-edit";
import { loadAiSnapshot, type AiContext, type AiConversationMode, type AiConversationType } from "./ai-runtime";
import { rpc, sha256 } from "./app-server-rpc";
import { writeClipboard } from "./clipboard";
import "./styles.css";
import { AddFolderIcon, ApplicationsIcon, AutomationIcon, BackArrowIcon, BackIcon, ChatBubbleIcon, CheckIcon, ChevronIcon, CloseIcon, ConfirmIcon, DiffIcon, ExternalOpenIcon, EyeIcon, EyeOffIcon, FileIcon, Folder, GlobalSearchIcon, HomeIcon, InfoIcon, MoreIcon, PanelToggleIcon, PortfolioIcon, RefreshIcon, RunsIcon, SearchIcon, SkillIcon, SwitcherIcon, TreeFolderIcon, WorkspaceIcon } from "./icons";
import { buildStatusChips, StatusChip, confirmationLabel, confirmationNeedsAction, conflictLabel, localChangeLabel, localStatusLabel, presenceLabel } from "./file-status";
import { VirtualFileTree, WorkspacePage, WorkspaceSwitcher, WorkspaceFileSearch } from "./workspace-page";
import { useAppServer, type AppServerStatus } from "./use-app-server";
import { useWorkbenchEventSignals } from "./use-workbench-event-signals";
import { clearStoredProjectDetail, readStoredPage, readStoredProjectDetail, writeStoredPage, writeStoredProjectDetail } from "./project-context-storage";
import { fileContextMenuItems } from "./file-context-menu";
import { describeServerError } from "./server-error";
import { useAgentIncidentReporting } from "./use-agent-incidents";
import { initialWorkspaceState, isPresentFile, isPreviewableFile, workspaceReducer } from "./workspace-state";
import { useNavigationGuards } from "./navigation-guards";
import { loadWorkspaceFileSnapshot } from "./workspace-file-pages";
import { LazyFileTree } from "./lazy-file-tree";
import { indexProgressLabel, indexWorkspacePath, isIndexActive, requestWorkspaceIndex, useWorkspaceIndexStatus } from "./workspace-index";
import { hydrateFileForRead } from "./file-hydration";
import { UserMenu, type LocalUserProfile } from "./user-menu";
import { applyTheme, readTheme } from "./theme";
import { loadLocalProfile, type LocalProfileStatus } from "./local-profile";
import { useHostPlatform, useWindowMaximized, WindowControls } from "./window-chrome";
import { CreateFolderDialog, currentDirectory } from "./create-folder-dialog";
import { LocalWorkspaceDialog, type WorkspaceAddProject } from "./local-workspace";
import { GlobalSearchPage } from "./global-search";
import type { ApplicationSearchOpenIntent } from "./applications";
import { hostPlatformUi, matchesGlobalSearchShortcut } from "./host-platform-ui";
import { DesktopUpdateControl } from "./desktop-update";
import cilaiSymbolUrl from "../../../assets/cilai-symbol-primary.svg";
// 页面一致性层必须排在所有组件样式之后引入。
import "./page-polish.css";

const ProjectProgressPage = lazy(() => import("./project-progress").then((module) => ({ default: module.ProjectProgressPage })));
const MyWorkPage = lazy(() => import("./my-work").then((module) => ({ default: module.MyWorkPage })));
const AiWorkbenchPage = lazy(() => import("./ai-workbench").then((module) => ({ default: module.AiWorkbenchPage })));
const ProjectPortfolioPage = lazy(() => import("./project-portfolio").then((module) => ({ default: module.ProjectPortfolioPage })));
const SettingsPage = lazy(() => import("./settings").then((module) => ({ default: module.SettingsPage })));
const AutomationsPage = lazy(() => import("./automations").then((module) => ({ default: module.AutomationsPage })));
const ApplicationsPage = lazy(() => import("./applications").then((module) => ({ default: module.ApplicationsPage })));
const SkillsPage = lazy(() => import("./skills").then((module) => ({ default: module.SkillsPage })));

interface ProjectDetailContext { projectId: string; displayName: string; primaryWorkspaceId: string | null; workspaceIds: string[]; selectedPhaseId: string | null; selectedPhaseName: string | null; }
const pageIds = ["mywork", "portfolio", "search", "automations", "skills", "workspace", "progress", "ai", "settings", "agents", "archived", "updates", "diagnostics", "disk-access", "usage", "tools", "data", "notifications", "applications"] as const;
type Page = (typeof pageIds)[number];
/** Guards the persisted page value, which is untrusted input from local storage. */
function isPage(value: string | null): value is Page {
  return value !== null && (pageIds as readonly string[]).includes(value);
}

function settingsSectionFor(page: Page): SettingsSection {
  return page === "agents" || page === "archived" || page === "updates" || page === "diagnostics" || page === "usage" || page === "tools" || page === "data" || page === "disk-access" || page === "notifications" ? page : "memory";
}

const selectedWorkspaceStorageKey = "cilai.workbench.selected-workspace";
const selectedFileStorageKeyPrefix = "cilai.workbench.selected-file:";
const showHiddenItemsStorageKey = "cilai.workbench.show-hidden-items";
const navigationExpandedStorageKey = "cilai.workbench.navigation-expanded";
const explorerWidthStorageKey = "cilai.workbench.explorer-width";
const defaultExplorerWidth = 300;
const minExplorerWidth = 240;
const maxExplorerWidth = 520;
const explorerKeyboardStep = 16;
/* info 档提示的停留时长。够读完一句中文，又不至于一直压在右下角。 */
const noticeAutoDismissMs = 4_000;

function clampExplorerWidth(width: number) {
  return Math.min(maxExplorerWidth, Math.max(minExplorerWidth, Math.round(width)));
}

function readExplorerWidth() {
  const stored = window.localStorage.getItem(explorerWidthStorageKey);
  if (stored === null || stored.trim() === "") return defaultExplorerWidth;
  const width = Number(stored);
  return Number.isFinite(width) ? clampExplorerWidth(width) : defaultExplorerWidth;
}

function selectedFileStorageKey(workspaceId: string) {
  return `${selectedFileStorageKeyPrefix}${workspaceId}`;
}

export function App() {
  useEffect(() => {
    // Outside editable text the operating system's menu only offers dictionary lookups and web
    // searches for whatever word happens to be under the cursor, which is never what the object
    // there can actually do. Editable fields keep the native menu so cut/copy/paste stay intact.
    const suppressNativeMenu = (event: MouseEvent) => {
      const target = event.target as HTMLElement | null;
      if (target?.closest("input, textarea, [contenteditable='true']")) return;
      event.preventDefault();
    };
    window.addEventListener("contextmenu", suppressNativeMenu);
    return () => window.removeEventListener("contextmenu", suppressNativeMenu);
  }, []);

  const hostPlatform = useHostPlatform();
  const platformUi = hostPlatformUi(hostPlatform);
  const windowMaximized = useWindowMaximized(hostPlatform === "windows");
  const [page, setPage] = useState<Page>("mywork");
  const [workspaceState, dispatchWorkspace] = useReducer(workspaceReducer, initialWorkspaceState);
  const { workspaces, selectedWorkspaceId, files, workspaceRevision, selectedFileId, conflicts, loadState, loadError } = workspaceState;
  const workspaceKinds = useRef(new Map<string, string>());
  workspaceKinds.current = new Map(workspaces.map((workspace) => [workspace.workspaceId, workspace.workspaceKind]));
  const [archiveNotice, setArchiveNotice] = useState<ArchiveNoticeOperation | null>(null);
  const archiveOperationSequence = useRef(0);
  const [archiveOpenIntent, setArchiveOpenIntent] = useState<{ conversationId: string; token: number } | null>(null);
  const [message, rawSetMessage] = useState<AppMessage | null>(null);
  // 所有提示都从这一个口子出去，调用方只多传一个 tone；失败走全局弹窗，其余走角落浮层。
  const setMessage = useCallback<MessageSink & ((value: null) => void)>(
    (value: string | null, tone: MessageTone = "info") => rawSetMessage(value === null ? null : { text: value, tone }),
    [],
  );

  const { status, restart: restartAppServer } = useAppServer(setMessage);
  /*
   * info 档说的都是"做成了"或者"正在做"，用户不需要因此做任何事，所以让它自己退场。
   * 之前它一直挂到下一条提示把它顶掉为止，一次扫描完成的提示能在界面上留一整天，
   * 想让它消失只能手动点一次"关闭"。error 档不走这里，失败必须由用户确认过才算看到。
   * 每来一条新提示都是新的对象，effect 因此会重新计时，而不是沿用上一条的剩余时间。
   */
  useEffect(() => {
    if (message?.tone !== "info") return;
    const timer = window.setTimeout(() => rawSetMessage(null), noticeAutoDismissMs);
    return () => window.clearTimeout(timer);
  }, [message]);
  const [connectingWorkspace, setConnectingWorkspace] = useState(false);
  const [fileQuery, setFileQuery] = useState("");
  const [workspaceSwitcherOpen, setWorkspaceSwitcherOpen] = useState(false);
  const [projectDetail, setProjectDetail] = useState<ProjectDetailContext | null>(null);
  const projectContextGeneration = useRef(0);
  const projectContextAtRender = projectContextGeneration.current;
  const [projectDeliverables, setProjectDeliverables] = useState<DeliverableItem[]>([]);
  const [showHiddenItems, setShowHiddenItems] = useState(() => window.localStorage.getItem(showHiddenItemsStorageKey) === "true");
  const [navigationExpanded, setNavigationExpanded] = useState(() => window.localStorage.getItem(navigationExpandedStorageKey) === "true");
  const [explorerWidth, setExplorerWidth] = useState(readExplorerWidth);
  const [resizingExplorer, setResizingExplorer] = useState(false);
  const [theme, setTheme] = useState(readTheme);
  const [openSkillId, setOpenSkillId] = useState<string | null>(null);
  const [recentConversations, setRecentConversations] = useState<RecentConversation[]>([]);
  const [recentConversationError, setRecentConversationError] = useState<unknown>(null);
  const [recentConversationRefresh, setRecentConversationRefresh] = useState(0);
  const [aiLaunchRequest, setAiLaunchRequest] = useState<AiLaunchRequest | null>(null);
  const [preferredAiConversationId, setPreferredAiConversationId] = useState<string | null>(null);
  const workspaceSwitcher = useRef<HTMLDivElement>(null);
  const explorerWidthRef = useRef(explorerWidth);
  const explorerResizeStart = useRef<{ clientX: number; width: number } | null>(null);
  const fileLoadGeneration = useRef(0);
  // Counts deliberate page changes so an in-flight restore never overrides where the user just went.
  // Captured once at mount: the persistence effect below writes the default page immediately,
  // so the restore must read the stored value before that first write lands.
  const initialStoredPageRef = useRef<Page | null | undefined>(undefined);
  if (initialStoredPageRef.current === undefined) {
    const raw = readStoredPage();
    initialStoredPageRef.current = isPage(raw) ? raw : null;
  }
  const [pendingFileTarget, setPendingFileTarget] = useState<string | null>(null);
  const [pendingFolderAction, setPendingFolderAction] = useState<WorkspaceSummary | null>(null);
  const [workspaceProjects, setWorkspaceProjects] = useState<Map<string, WorkspaceProjectIdentity[]>>(new Map());
  const workspaceProjectIndex = useMemo(() => unambiguousWorkspaceProjects(workspaceProjects), [workspaceProjects]);
  const [aiProjects, setAiProjects] = useState<AiProjectOption[]>([]);
  const [aiProjectCatalogError, setAiProjectCatalogError] = useState("");
  const [aiProjectCatalogReady, setAiProjectCatalogReady] = useState(false);
  const [aiProjectRefreshToken, setAiProjectRefreshToken] = useState(0);
  const [aiProjectCreateOpen, setAiProjectCreateOpen] = useState(false);
  const [workspaceAddOpen, setWorkspaceAddOpen] = useState(false);
  const [projectCreateForFolder, setProjectCreateForFolder] = useState(false);
  const [localProfile, setLocalProfile] = useState<LocalProfileStatus | null>(null);
  const [localProfileError, setLocalProfileError] = useState<string | null>(null);
  const [applicationSearchOpenIntent, setApplicationSearchOpenIntent] = useState<ApplicationSearchOpenIntent | null>(null);
  const [automationOpenIntent, setAutomationOpenIntent] = useState<{ruleId:string;token:string}|null>(null);
  const [notificationOpenIntent, setNotificationOpenIntent] = useState<PersonalNotificationOpen|null>(null);
  const [portfolioCreateIntent, setPortfolioCreateIntent] = useState(0);
  const [portfolioDeleteProjectId, setPortfolioDeleteProjectId] = useState<string | null>(null);
  const [myWorkRefreshToken, setMyWorkRefreshToken] = useState(0);

  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  const applyExplorerWidth = useCallback((width: number, persist: boolean) => {
    const next = clampExplorerWidth(width);
    explorerWidthRef.current = next;
    setExplorerWidth(next);
    if (persist) window.localStorage.setItem(explorerWidthStorageKey, String(next));
  }, []);

  useEffect(() => {
    if (!resizingExplorer) return;
    const move = (event: PointerEvent) => {
      const start = explorerResizeStart.current;
      if (!start) return;
      applyExplorerWidth(start.width + event.clientX - start.clientX, false);
    };
    const finish = () => {
      window.localStorage.setItem(explorerWidthStorageKey, String(explorerWidthRef.current));
      explorerResizeStart.current = null;
      setResizingExplorer(false);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", finish);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", finish);
    };
  }, [applyExplorerWidth, resizingExplorer]);

  const startExplorerResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    explorerResizeStart.current = { clientX: event.clientX, width: explorerWidthRef.current };
    setResizingExplorer(true);
    event.preventDefault();
  };

  const resizeExplorerWithKeyboard = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    let next: number | null = null;
    if (event.key === "ArrowLeft") next = explorerWidthRef.current - explorerKeyboardStep;
    if (event.key === "ArrowRight") next = explorerWidthRef.current + explorerKeyboardStep;
    if (event.key === "Home") next = minExplorerWidth;
    if (event.key === "End") next = maxExplorerWidth;
    if (next === null) return;
    event.preventDefault();
    applyExplorerWidth(next, true);
  };

  const selectedWorkspace = workspaces.find((item) => item.workspaceId === selectedWorkspaceId) ?? null;
  const displayedFiles = useMemo(() => showHiddenItems ? files : files.filter((file) => !isHiddenPath(file.relativePath)), [files, showHiddenItems]);
  const selectedFile = displayedFiles.find((item) => item.localFileId === selectedFileId) ?? null;
  const selectedProjectIdentity = selectedWorkspace
    ? projectDetail?.workspaceIds.includes(selectedWorkspace.workspaceId) ? projectDetail : workspaceProjectIndex.get(selectedWorkspace.workspaceId) ?? null
    : null;
  const workspaceOperationBusy = connectingWorkspace;
  const [selectedDirectory, setSelectedDirectory] = useState<{ workspaceId: string; path: string } | null>(null);
  const [folderCreation, setFolderCreation] = useState<{ workspace: WorkspaceSummary; parentRelativePath: string } | null>(null);
  const fileDeliverableLookup = useMemo(() => {
    const lookup = new Map<string, { deliverableTitle: string; versionNo: number; accepted: boolean }>();
    for (const deliverable of projectDeliverables) {
      for (const version of deliverable.versions) {
        for (const binding of version.bindings) {
          if (!lookup.has(binding.localFileId)) {
            lookup.set(binding.localFileId, {
              deliverableTitle: deliverable.title,
              versionNo: version.versionNo,
              accepted: version.versionId === deliverable.currentAcceptedVersionId,
            });
          }
        }
      }
    }
    return lookup;
  }, [projectDeliverables]);

  // Navigation keeps the current project: pages either work inside it (推进/文件/AI 工作台) or sit
  // above it (工作台/项目总览/变更历史/搜索), and in both cases the project context is kept so
  // the user would return to. Only exitProject, switching workspace, or opening a different project
  // ends the context. Latching the restore ref cancels a restore that has not run yet, so navigating
  // while the local service is still starting is not overridden a moment later.
  const guards = useNavigationGuards();

  const navigateTo = useCallback((next: Page) => {
    guards.userNavigated();
    setPage(next);
  }, [guards]);

  const openRunConversation = useCallback(async (runId: string) => {
    try {
      const snapshot = await loadAiSnapshot();
      const run = snapshot.runs.find((item) => item.runId === runId);
      if (!run) {
        setMessage("未找到对应对话。", "error");
        return;
      }
      setPreferredAiConversationId(run.conversationId);
      navigateTo("ai");
    } catch (error) {
      setMessage(String(error), "error");
    }
  }, [navigateTo, setMessage]);

  useEffect(() => {
    let current = true;
    let stop: (() => void) | undefined;
    void listen<PersonalNotificationOpen>(personalNotificationOpenEvent, (payload) => {
      if (current) setNotificationOpenIntent(payload);
    }).then(async (release) => {
      if (!current) { release(); return; }
      stop = release;
      if (localProfile?.profileId) await invoke("personal_notification_navigation_ready", { profileId: localProfile.profileId });
    }).catch(() => { if (current) setMessage("通知定位尚未就绪。", "error"); });
    return () => { current = false; stop?.(); };
  }, [localProfile?.profileId, setMessage]);

  useEffect(() => {
    if (!notificationOpenIntent || !localProfile?.profileId) return;
    setNotificationOpenIntent(null);
    try {
      const target = personalNotificationTarget(notificationOpenIntent, localProfile.profileId);
      if (target.kind === "run") { void openRunConversation(target.runId); return; }
      if (target.kind === "automation") {
        setAutomationOpenIntent({ ruleId: target.automationRuleId, token: notificationOpenIntent.notificationId });
        navigateTo("automations"); return;
      }
      setApplicationSearchOpenIntent({ token: notificationOpenIntent.notificationId, kind: "application", applicationId: target.creationProjectId,
        objectType: "personal-page-notification", objectId: target.documentUrl, target: "existing-document" });
      navigateTo("applications");
    } catch { setMessage("通知对应的内容已不可用。", "error"); }
  }, [notificationOpenIntent, localProfile?.profileId, openRunConversation, navigateTo, setMessage]);

  /** The single deliberate way to leave a project; also drops the restore record. */
  const exitProject = useCallback((next: Page = "portfolio") => {
    projectContextGeneration.current += 1;
    guards.projectExited();
    clearStoredProjectDetail();
    setProjectDetail(null);
    setPage(next);
  }, []);

  useEffect(() => {
    invalidateArchiveOperations();
    setArchiveNotice(null);
    setArchiveOpenIntent(null);
  }, [localProfile?.profileId, localProfile?.profileId]);
  useEffect(() => {
    const changed = (event: Event) => {
      const result = (event as CustomEvent<ConversationArchiveChange>).detail;
      setMyWorkRefreshToken((current) => current + 1);
      if (!result.archived) return;
      const archived = new Set(result.conversations.map((item) => item.conversationId));
      setPreferredAiConversationId((current) => current && archived.has(current) ? null : current);
      if (result.changedConversationIds.length) {
        setArchiveNotice({ id: ++archiveOperationSequence.current, conversationIds: result.changedConversationIds });
      }
    };
    window.addEventListener(conversationArchiveChanged, changed);
    return () => window.removeEventListener(conversationArchiveChanged, changed);
  }, []);

  const profileLoadGeneration = useRef(0);
  const refreshLocalProfile = useCallback(async () => {
    const generation = ++profileLoadGeneration.current;
    setLocalProfile(null);
    setLocalProfileError(null);
    try { const profile = await loadLocalProfile(); if (generation === profileLoadGeneration.current) setLocalProfile(profile); }
    catch (error) { if (generation === profileLoadGeneration.current) setLocalProfileError(String(error)); }
  }, []);
  useEffect(() => {
    if (status?.phase === "running" && !status.runtimeInitializationError) void refreshLocalProfile();
    else { profileLoadGeneration.current += 1; setLocalProfile(null); if (status?.runtimeInitializationError) setLocalProfileError(status.runtimeInitializationError); }
  }, [refreshLocalProfile, status?.phase, status?.serverInstanceId, status?.runtimeInitializationError]);
  useEffect(() => {
    const pinApplicationViewport = () => {
      if (window.scrollX !== 0 || window.scrollY !== 0) window.scrollTo(0, 0);
    };
    pinApplicationViewport();
    window.addEventListener("scroll", pinApplicationViewport, { passive: true });
    return () => window.removeEventListener("scroll", pinApplicationViewport);
  }, []);
  useEffect(() => {
    const openSearch = (event: KeyboardEvent) => {
      if (matchesGlobalSearchShortcut(hostPlatform, event)) {
        event.preventDefault();
        navigateTo("search");
      }
    };
    window.addEventListener("keydown", openSearch);
    return () => window.removeEventListener("keydown", openSearch);
  }, [hostPlatform, navigateTo]);

  const loadWorkspaces = useCallback(async () => {
    try {
      const result = await rpc<WorkspaceListResult>("workspace/list");
      dispatchWorkspace({
        type: "workspacesLoaded",
        workspaces: result.workspaces,
        persistedId: window.localStorage.getItem(selectedWorkspaceStorageKey),
      });
    } catch (error) {
      setMessage(String(error), "error"); dispatchWorkspace({ type: "workspaceListFailed", error });
    }
  }, []);

  const loadFiles = useCallback(async (workspaceId: string) => {
    if (workspaceKinds.current.get(workspaceId) !== "local") throw new Error("个人版不支持此云端来源，未读取文件。");
    const generation = ++fileLoadGeneration.current;
    try {
      const snapshot = await loadWorkspaceFileSnapshot(workspaceId, () => generation === fileLoadGeneration.current);
      if (snapshot === null || generation !== fileLoadGeneration.current) return;
      dispatchWorkspace({
        type: "filesLoaded",
        files: snapshot.files,
        revision: snapshot.revision,
        persistedId: window.localStorage.getItem(selectedFileStorageKey(workspaceId)),
      });
    } catch (error) {
      if (generation === fileLoadGeneration.current) throw error;
    }
  }, []);

  // 后台索引完成后刷新引用和内容状态，目录结构始终读取本地磁盘。
  const [indexRefreshToken, setIndexRefreshToken] = useState(0);
  const [indexingPath, setIndexingPath] = useState<string | null>(null);
  const [unindexedFileDownloading, setUnindexedFileDownloading] = useState(false);
  const unindexedFileRead = useRef<AbortController | null>(null);
  useEffect(() => () => { unindexedFileRead.current?.abort(); }, [page, selectedWorkspaceId, localProfile?.profileId]);
  /** 逐层浏览时，新建或删除之后要重读已展开的各层。 */
  const [layeredTreeRefresh, setLayeredTreeRefresh] = useState(0);
  const localWorkspaceId = selectedWorkspace?.workspaceKind === "local" ? selectedWorkspace.workspaceId : null;
  const workspaceIndex = useWorkspaceIndexStatus(localWorkspaceId, status?.phase === "running", () => {
    void loadWorkspaces();
    setLayeredTreeRefresh((value) => value + 1);
    if (localWorkspaceId) void loadFiles(localWorkspaceId).catch((error) => setMessage(String(error), "error"));
  }, indexRefreshToken);
  const startWorkspaceIndex = async (workspaceId: string) => {
    await requestWorkspaceIndex(workspaceId);
    setIndexRefreshToken((value) => value + 1);
  };
  /** 索引还没收进来的文件：先单独补录它，再像普通文件一样选中预览。 */
  const openUnindexedFile = async (workspaceId: string, relativePath: string) => {
    if (unindexedFileRead.current) return;
    const controller = new AbortController();
    unindexedFileRead.current = controller;
    setIndexingPath(relativePath);
    try {
      await hydrateFileForRead(workspaceId, relativePath, controller.signal, () => setUnindexedFileDownloading(true));
      controller.signal.throwIfAborted();
      const indexed = await indexWorkspacePath(workspaceId, relativePath);
      controller.signal.throwIfAborted();
      await loadFiles(workspaceId);
      controller.signal.throwIfAborted();
      setSelectedDirectory(null);
      dispatchWorkspace({ type: "fileSelected", localFileId: indexed.localFileId });
    } catch (error) {
      if (!controller.signal.aborted) setMessage(`无法打开「${relativePath}」：${describeServerError(error)}`, "error");
    } finally {
      if (unindexedFileRead.current === controller) {
        unindexedFileRead.current = null;
        setIndexingPath(null);
        setUnindexedFileDownloading(false);
      }
    }
  };

  // 失败的运行补记成工作台里的关注事项。放在应用外壳而不是 AI 页面里：
  // 用户多半正待在工作台，而不是盯着运行页看它失败。
  useAgentIncidentReporting({
    workspaceRoot: selectedWorkspace?.workspaceKind === "local" ? selectedWorkspace.localRoot : null,
    onReported: () => setMyWorkRefreshToken((value) => value + 1),
    onError: (message) => setMessage(message, "error"),
  });

  const eventSignals = useWorkbenchEventSignals({
    active: status?.phase === "running" && !workspaceOperationBusy,
    selectedWorkspaceId,
    serverInstanceId: status?.serverInstanceId ?? null,
    onSelectedWorkspaceChanged: () => { setLayeredTreeRefresh((value) => value + 1); if (selectedWorkspaceId) void loadFiles(selectedWorkspaceId).catch((error) => setMessage(String(error), "error")); },
    onMessage: setMessage,
  });

  useEffect(() => { if (status?.phase === "running") void loadWorkspaces(); }, [loadWorkspaces, status?.phase, status?.serverInstanceId]);
  useEffect(() => {
    if (status?.phase !== "running" || workspaceOperationBusy) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const refresh = async () => {
      if (document.visibilityState !== "hidden") await loadWorkspaces();
      if (!cancelled) timer = setTimeout(() => void refresh(), 3000);
    };
    const onFocus = () => { void loadWorkspaces(); setLayeredTreeRefresh((value) => value + 1); };
    timer = setTimeout(() => void refresh(), 3000);
    window.addEventListener("focus", onFocus);
    return () => { cancelled = true; clearTimeout(timer); window.removeEventListener("focus", onFocus); };
  }, [loadWorkspaces, status?.phase, workspaceOperationBusy]);
  useEffect(() => {
    // 清理已经由 workspaceSelected 那个 case 做掉了；这里只需作废在途的加载。
    fileLoadGeneration.current += 1;
    if (selectedWorkspaceId) void loadFiles(selectedWorkspaceId).catch((error) => setMessage(String(error), "error"));
  }, [loadFiles, selectedWorkspaceId]);
  useEffect(() => {
    if (loadState !== "ready") return;
    if (selectedWorkspaceId) window.localStorage.setItem(selectedWorkspaceStorageKey, selectedWorkspaceId);
    else window.localStorage.removeItem(selectedWorkspaceStorageKey);
  }, [loadState, selectedWorkspaceId]);
  useEffect(() => {
    if (!selectedWorkspaceId || !selectedFileId) return;
    window.localStorage.setItem(selectedFileStorageKey(selectedWorkspaceId), selectedFileId);
  }, [selectedFileId, selectedWorkspaceId]);
  useEffect(() => {
    window.localStorage.setItem(showHiddenItemsStorageKey, String(showHiddenItems));
  }, [showHiddenItems]);
  useEffect(() => {
    window.localStorage.setItem(navigationExpandedStorageKey, String(navigationExpanded));
  }, [navigationExpanded]);
  useEffect(() => {
    dispatchWorkspace({ type: "visibleFilesChanged", visible: displayedFiles });
  }, [displayedFiles]);
  useEffect(() => {
    if (!pendingFileTarget) return;
    if (displayedFiles.some((item) => item.localFileId === pendingFileTarget)) {
      dispatchWorkspace({ type: "fileSelected", localFileId: pendingFileTarget });
      setPendingFileTarget(null);
    }
  }, [pendingFileTarget, displayedFiles]);
  useEffect(() => {
    if (!workspaceSwitcherOpen) return;
    const closeOnOutsidePointer = (event: PointerEvent) => {
      if (!workspaceSwitcher.current?.contains(event.target as Node)) setWorkspaceSwitcherOpen(false);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setWorkspaceSwitcherOpen(false);
    };
    window.addEventListener("pointerdown", closeOnOutsidePointer);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("pointerdown", closeOnOutsidePointer);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [workspaceSwitcherOpen]);
  useEffect(() => {
    if (!projectDetail) { setProjectDeliverables([]); return; }
    let cancelled = false;
    void rpc<DeliverableListResult>("deliverable/list", { projectId: projectDetail.projectId })
      .then((result) => { if (!cancelled) setProjectDeliverables(result.deliverables); })
      .catch((error) => { if (!cancelled) setMessage(String(error), "error"); });
    return () => { cancelled = true; };
  }, [projectDetail?.projectId]);

  /**
   * Adds another local folder to the project the user is currently inside. A project is an
   * abstract container; folders are where it actually lives on disk, and a folder belongs to
   * exactly one project.
   */
  const [editTarget, setEditTarget] = useState<ProjectEditTarget | null>(null);

  /** Reads the project fresh so the editor edits the revision it is actually showing. */
  const openProjectEditor = async (projectId: string) => {
    try {
      const result = await rpc<ProjectGetResult>("project/get", { projectId });
      setEditTarget({
        projectId: result.project.projectId,
        displayName: result.project.displayName,
        goal: result.project.goal,
        stateRevision: result.project.stateRevision,
        workspaceIds: result.workspaceIds,
      });
    } catch (error) {
      setMessage(`无法读取项目信息：${String(error)}`, "error");
    }
  };

  /** The project whose folders the folder actions apply to: the editor's, or the one you are in. */
  const folderOwner = editTarget ?? projectDetail;

  const addFolderToProject = async (owner: WorkspaceAddProject | ProjectEditTarget | ProjectDetailContext | null = folderOwner) => {
    if (workspaceOperationBusy || !owner) return;
    setConnectingWorkspace(true);
    try {
      const localRoot = await invoke<string | null>("workspace_pick_directory");
      if (!localRoot) return;

      // 已连接的目录可直接关联项目，服务端也按实际目录身份复用登记。
      const already = workspaces.find((item) => sameLocalRoot(item.localRoot, localRoot));
      if (already && (owner.workspaceIds ?? []).includes(already.workspaceId)) {
        setMessage(`「${already.displayName}」已经在「${owner.displayName}」里了。`);
        return;
      }

      const displayName = localRoot.split(/[\\/]/).filter(Boolean).at(-1) ?? "文件夹";
      setMessage(already ? "正在把文件夹加入项目…" : "正在连接文件夹…");
      const created = already
        ? { workspace: already }
        : await rpc<WorkspaceCreateResult>("workspace/create", { localRoot, displayName, clientOperationId: `desktop-workspace-${randomUuid()}` });
      const requestFingerprint = await sha256([owner.projectId, created.workspace.workspaceId].join("\n"));
      const attached = await rpc<ProjectAttachWorkspaceResult>("project/attachWorkspace", {
        projectId: owner.projectId,
        workspaceId: created.workspace.workspaceId,
        clientOperationId: `desktop-attach-${randomUuid()}`,
        requestFingerprint,
      });
      setProjectDetail((current) => current?.projectId === owner.projectId ? { ...current, workspaceIds: attached.workspaceIds } : current);
      setEditTarget((current) => current?.projectId === owner.projectId ? { ...current, stateRevision: attached.project.stateRevision, workspaceIds: attached.workspaceIds } : current);
      await loadWorkspaces();
      setAiProjectRefreshToken((value) => value + 1);
      dispatchWorkspace({ type: "workspaceSelected", workspaceId: created.workspace.workspaceId });
      const joined = `文件夹已加入「${owner.displayName}」，该项目现有 ${attached.workspaceIds.length} 个文件夹。`;
      if (already) { setMessage(joined); return; }
      // 首次索引交给后台：大文件夹要读完所有文件内容，同步等它会让整个界面卡住。
      // 索引期间文件树逐层浏览，进度显示在文件树顶部。
      try {
        await startWorkspaceIndex(created.workspace.workspaceId);
        setMessage(`${joined}正在后台建立索引，期间可以直接浏览。`);
      } catch (error) {
        setMessage(`${joined}但后台索引没有启动：${describeServerError(error)}`, "error");
      }
    } catch (error) {
      setMessage(describeServerError(error), "error");
    } finally {
      setConnectingWorkspace(false);
    }
  };

  /** Removes a folder from the current project so it can be claimed elsewhere. */
  const removeFolderFromProject = async (workspace: WorkspaceSummary) => {
    if (workspaceOperationBusy || !folderOwner) return;
    setConnectingWorkspace(true);
    try {
      const requestFingerprint = await sha256([folderOwner.projectId, workspace.workspaceId].join("\n"));
      const result = await rpc<ProjectDetachWorkspaceResult>("project/detachWorkspace", {
        projectId: folderOwner.projectId,
        workspaceId: workspace.workspaceId,
        clientOperationId: `desktop-detach-${randomUuid()}`,
        requestFingerprint,
      });
      setProjectDetail((current) => current?.projectId === folderOwner.projectId ? { ...current, primaryWorkspaceId: result.project.primaryWorkspaceId, workspaceIds: result.workspaceIds } : current);
      setEditTarget((current) => current?.projectId === folderOwner.projectId ? { ...current, stateRevision: result.project.stateRevision, workspaceIds: result.workspaceIds } : current);
      if (selectedWorkspaceId === workspace.workspaceId) dispatchWorkspace({ type: "workspaceSelected", workspaceId: result.project.primaryWorkspaceId });
      await loadWorkspaces();
      setMessage(`「${workspace.displayName}」已移出项目；应用内索引、扫描结果、历史和磁盘文件均已保留。`);
    } catch (error) {
      setMessage(String(error), "error");
    } finally {
      setConnectingWorkspace(false);
    }
  };

  const createWorkspace = async () => {
    if (workspaceOperationBusy) return;
    setConnectingWorkspace(true);
    try {
      const localRoot = await invoke<string | null>("workspace_pick_directory");
      if (!localRoot) return;
      const displayName = localRoot.split(/[\\/]/).filter(Boolean).at(-1) ?? "本地工作区";
      setMessage("正在校验并连接目录…");
      const existing = workspaces.find((workspace) => workspace.localRoot === localRoot);
      const created = existing ? { workspace: existing } : await rpc<WorkspaceCreateResult>("workspace/create", { localRoot, displayName, clientOperationId: `desktop-workspace-${randomUuid()}` });
      await loadWorkspaces();
      dispatchWorkspace({ type: "workspaceSelected", workspaceId: created.workspace.workspaceId });
      // 首次索引交给后台，不同步等待；目录出现在文件树里本身就是回执，进度在文件树顶部。
      try {
        await startWorkspaceIndex(created.workspace.workspaceId);
        setMessage(null);
      } catch (error) {
        setMessage(`目录已连接，但后台索引没有启动：${describeServerError(error)}`, "error");
      }
      return created.workspace;
    } catch (error) { setMessage(describeServerError(error), "error"); }
    finally { setConnectingWorkspace(false); }
  };

  const scan = async () => {
    if (!selectedWorkspaceId || workspaceOperationBusy) return;
    if (selectedWorkspace?.workspaceKind !== "local") { setMessage("个人版不支持此云端来源。", "error"); return; }
    try { await startWorkspaceIndex(selectedWorkspaceId); setMessage("已开始扫描。"); }
    catch (error) { setMessage(`重新扫描没有启动：${describeServerError(error)}`, "error"); }
  };

  const revealWorkspace = async (workspace: WorkspaceSummary) => {
    if (workspace.workspaceKind === "cloud_team") {
      setMessage("个人版不支持此云端来源。", "error");
      return;
    }
    if (!platformUi) {
      setMessage("当前平台不支持文件定位。", "error");
      return;
    }
    try {
      await invoke("workspace_reveal_directory", { localRoot: workspace.localRoot });
    } catch (error) {
      setMessage(`无法在${platformUi.fileManagerName}中显示工作区：${String(error)}`, "error");
    }
  };

  const selectWorkspace = (workspaceId: string) => {
    if (workspaceOperationBusy) return;
    unindexedFileRead.current?.abort();
    dispatchWorkspace({ type: "workspaceSelected", workspaceId: workspaceId });
    setFileQuery("");
    setWorkspaceSwitcherOpen(false);
    exitProject("workspace");
  };

  // 【未完成】冲突解决的调用链路，当前不接任何界面入口。
  //
  // 云端平台尚未上线，放开之前必须先补两处：其一，选择采用平台版本后没有任何代码把云端
  // 内容写到磁盘；其二，这里记录的决策 checksum 与人工确认时实际算出的 checksum 从不比对，
  // 会留下"记录说采用了平台、磁盘上还是本地那份"的静默错账。
  // 完整方案见《文件同步确认与冲突状态模型》第 19 节。
  const navItems = useMemo(() => ([
    { id: "mywork" as const, label: "工作台", icon: <HomeIcon/> },
    { id: "ai" as const, label: "对话", icon: <ChatBubbleIcon/> },
    { id: "portfolio" as const, label: "项目", icon: <PortfolioIcon/> },
    { id: "workspace" as const, label: "文件", icon: <WorkspaceIcon/> },
    { id: "automations" as const, label: "自动化", icon: <AutomationIcon/> },
    { id: "skills" as const, label: "技能", icon: <SkillIcon/> },
    { id: "applications" as const, label: "应用", icon: <ApplicationsIcon/> },
  ]), []);
  const selectFile = (id: string) => { unindexedFileRead.current?.abort(); setSelectedDirectory(null); dispatchWorkspace({ type: "fileSelected", localFileId: id }); setPage("workspace"); };
  const openWorkspaceFile = (localFileId: string, workspaceId: string) => {
    unindexedFileRead.current?.abort();
    setSelectedDirectory(null);
    if (workspaceId !== selectedWorkspaceId) dispatchWorkspace({ type: "workspaceSelected", workspaceId });
    setPendingFileTarget(localFileId);
    setPage("workspace");
  };
  const openWorkspacePath = async (relativePath: string, workspaceId: string) => {
    unindexedFileRead.current?.abort();
    const generation = ++fileLoadGeneration.current;
    try {
      const normalizedPath = relativePath.replaceAll("\\", "/");
      // 本地文件只补录这一条：整个工作区重扫一遍要读完所有文件内容，AI 刚写出的一个
      // 文件不该让界面等那么久。云端空间仍按原样刷新。
      const local = workspaces.find((item) => item.workspaceId === workspaceId)?.workspaceKind === "local";
      let indexedId: string | null = null;
      if (local) indexedId = (await indexWorkspacePath(workspaceId, normalizedPath)).localFileId;
      else { setMessage("个人版不支持此云端来源。", "error"); return false; }
      const snapshot = await loadWorkspaceFileSnapshot(workspaceId, () => generation === fileLoadGeneration.current);
      if (!snapshot || generation !== fileLoadGeneration.current) return false;
      const file = snapshot.files.find((candidate) => indexedId !== null
        ? candidate.localFileId === indexedId
        : candidate.relativePath.replaceAll("\\", "/") === normalizedPath);
      if (!file || !isPreviewableFile(file)) return false;
      if (workspaceId !== selectedWorkspaceId) dispatchWorkspace({ type: "workspaceSelected", workspaceId });
      dispatchWorkspace({ type: "filesLoaded", files: snapshot.files, revision: snapshot.revision, persistedId: null });
      setSelectedDirectory(null);
      dispatchWorkspace({ type: "fileSelected", localFileId: file.localFileId });
      setPendingFileTarget(null);
      setPage("workspace");
      return true;
    } catch {
      return false;
    }
  };

  const [contextMenu, setContextMenu] = useState<ContextMenuState>(null);
  /** Bumped by the file menu so the document stage opens on the diff instead of the preview. */
  const [fileDiffIntent, setFileDiffIntent] = useState(0);
  /** Bumped by the file menu to open the detail drawer, where confirmation and its history live. */
  const [fileDetailIntent, setFileDetailIntent] = useState(0);
  const [pendingFileDelete, setPendingFileDelete] = useState<WorkspaceFileItem | null>(null);

  /**
   * On the Files page a delete removes the item from disk. It goes to the system trash, so a
   * mistake stays recoverable, and the index is rescanned afterwards so the tree matches reality.
   */
  const deleteWorkspaceItem = async (file: WorkspaceFileItem) => {
    if (!selectedWorkspace || workspaceOperationBusy) return;
    if (selectedWorkspace.workspaceKind !== "local") { setMessage("个人版不支持此云端来源。", "error"); return; }
    setConnectingWorkspace(true);
    try {
      await rpc("workspace/targetValidate", {
        workspaceId: selectedWorkspace.workspaceId,
        relativePath: file.relativePath,
        requireWrite: true,
      });
      await invoke("workspace_delete_item", {
        workspaceId: selectedWorkspace.workspaceId,
        localRoot: selectedWorkspace.localRoot,
        relativePath: file.relativePath,
      });
      if (selectedFileId === file.localFileId) dispatchWorkspace({ type: "fileSelected", localFileId: null });
      setMessage(platformUi
        ? `「${file.relativePath}」已移到${platformUi.trashName}。`
        : `「${file.relativePath}」已删除。`);
      // 本地删除后由后台索引重扫，索引更新完成时文件树随之刷新；云端仍同步刷新。
      setLayeredTreeRefresh((value) => value + 1); await startWorkspaceIndex(selectedWorkspace.workspaceId);
    } catch (error) {
      setMessage(`删除失败：${String(error)}`, "error");
    } finally {
      setConnectingWorkspace(false);
    }
  };
  const openItemInSystem = async (relativePath: string, openMode: "default" | "finder") => {
    if (!selectedWorkspace) return;
    if (selectedWorkspace.workspaceKind === "cloud_team") {
      setMessage("个人版不支持此云端来源。", "error");
      return;
    }
    if (openMode === "finder" && !platformUi) {
      setMessage("当前平台不支持文件定位。", "error");
      return;
    }
    try {
      await invoke("workspace_open_item", { localRoot: selectedWorkspace.localRoot, relativePath, openMode });
    } catch (error) {
      setMessage(`无法打开：${String(error)}`, "error");
    }
  };
  const copyToClipboard = async (value: string, what: string) => {
    try {
      await writeClipboard(value);
      setMessage(`${what}已复制。`);
    } catch (error) {
      setMessage(`复制失败：${String(error)}`, "error");
    }
  };

  const openFileContextMenu = (event: React.MouseEvent, file: WorkspaceFileItem) => {
    if (!selectedWorkspace) return;
    const openDetail = (target: WorkspaceFileItem) => { selectFile(target.localFileId); setFileDetailIntent((value) => value + 1); };
    setContextMenu(openContextMenu(event, fileContextMenuItems(file, selectedWorkspace, workspaceOperationBusy, {
      preview: (target) => selectFile(target.localFileId),
      showDiff: (target) => { selectFile(target.localFileId); setFileDiffIntent((value) => value + 1); },
      openDetail,
      discussWithAgent: (target) => {
        selectFile(target.localFileId);
        openFileAi(target);
      },
      copy: (value, what) => void copyToClipboard(value, what),
      openInSystem: (relativePath, openMode) => void openItemInSystem(relativePath, openMode),
      rescan: () => void scan(),
      requestDelete: (target) => setPendingFileDelete(target),
    }, hostPlatform)));
  };

  /** Resolves project identity and applies it, unless the user navigated away while it was in flight. */
  const applyProjectDetail = useCallback(async (projectId: string, phaseId: string | null) => {
    const stillCurrent = guards.beginProjectResolve();
    const result = await rpc<ProjectGetResult>("project/get", { projectId });
    if (!stillCurrent()) return false;
    dispatchWorkspace({ type: "workspaceSelected", workspaceId: result.project.primaryWorkspaceId });
    setProjectDetail({
      projectId: result.project.projectId,
      displayName: result.project.displayName,
      primaryWorkspaceId: result.project.primaryWorkspaceId,
      workspaceIds: result.workspaceIds,
      selectedPhaseId: phaseId,
      selectedPhaseName: null,
    });
    return true;
  }, []);

  const openProjectDetail = useCallback((projectId: string, options?: { phaseId?: string | null }) => {
    void (async () => {
      try {
        if (await applyProjectDetail(projectId, options?.phaseId ?? null)) setPage("progress");
      } catch (error) {
        setMessage(String(error), "error");
      }
    })();
  }, [applyProjectDetail, guards]);

  useEffect(() => {
    // Only ever writes; clearing belongs to exitProject, the single deliberate way to leave a
    // project. That split keeps this from wiping the stored value on first mount, when
    // projectDetail is still null because the restore below has not run yet.
    if (projectDetail && projectContextAtRender === projectContextGeneration.current) {
      writeStoredProjectDetail({ projectId: projectDetail.projectId, phaseId: projectDetail.selectedPhaseId });
    }
  }, [projectDetail, projectContextAtRender]);
  useEffect(() => {
    writeStoredPage(page);
  }, [page]);
  useEffect(() => {
    let cancelled = false;
    setAiProjectCatalogReady(false);
    setAiProjectCatalogError("");
    void rpc<ProjectPortfolioListResult>("projectPortfolio/list", { workspaceIds: [] })
      .then(async (result) => {
        const projects = result.items.map((item): AiProjectOption => {
          const primaryWorkspace = item.workspaceId ? workspaces.find((workspace) => workspace.workspaceId === item.workspaceId) ?? null : null;
          if (item.workspaceId && !primaryWorkspace) throw new Error(`项目 ${item.projectId} 引用了未连接的文件夹 ${item.workspaceId}`);
          return {
            projectId: item.projectId,
            displayName: item.displayName,
            primaryWorkspaceId: item.workspaceId,
            primaryWorkspaceName: item.workspaceDisplayName,
            primaryWorkspaceRoot: primaryWorkspace?.workspaceKind === "local" ? primaryWorkspace.localRoot : null,
            primaryWorkspaceKind: primaryWorkspace?.workspaceKind ?? null,
            primaryTeamSpaceId: primaryWorkspace?.teamSpaceId ?? null,
            workspaceIds: item.workspaceIds,
          };
        });
        if (cancelled) return;
        setAiProjects(projects);
        setWorkspaceProjects(indexWorkspaceProjects(projects));
        setAiProjectCatalogReady(true);
      })
      .catch((reason) => { if (!cancelled) { setWorkspaceProjects(new Map()); setAiProjects([]); setAiProjectCatalogReady(false); setAiProjectCatalogError(String(reason)); } });
    return () => { cancelled = true; };
  }, [workspaces, eventSignals.nextSequence, aiProjectRefreshToken]);
  useEffect(() => {
    if (status?.phase !== "running" || loadState !== "ready") return;
    if (!guards.claimRestore()) return;
    const restoredPage = initialStoredPageRef.current ?? null;
    // 读不出来时 readStoredProjectDetail 会顺手把坏数据清掉，留着只会每次启动都失败一次。
    const parsed = readStoredProjectDetail();
    if (!parsed) {
      // No project to restore, but the user may still have been on a cross-project page.
      if (restoredPage) setPage(restoredPage);
      return;
    }
    const userMoved = guards.watchUserNavigation();
    void applyProjectDetail(parsed.projectId, parsed.phaseId)
      // 上下文照样恢复（它不改变用户看到的东西），但页面必须听用户的。
      .then((applied) => { if (applied && !userMoved()) setPage(restoredPage ?? "progress"); })
      .catch(() => clearStoredProjectDetail());
  }, [applyProjectDetail, guards, status?.phase, loadState]);

  useEffect(() => {
    if (page !== "mywork" || status?.phase !== "running") return;
    let active = true;
    setRecentConversations([]);
    setRecentConversationError(null);
    void invoke<RecentConversation[]>("ai_recent_conversations").then((items) => { if (active) setRecentConversations(items); })
      .catch((error) => { if (active) setRecentConversationError(error); });
    return () => { active = false; };
  }, [page, status?.phase, localProfile?.profileId, recentConversationRefresh]);

  const openWorkItem = (item: MyWorkItem) => openProjectDetail(item.projectId, { phaseId: item.phaseId });

  const openSearchItem = (item: UnifiedSearchItem) => {
    if (item.objectType === "conversation" && ["native-conversations", "native-archived-conversations"].includes(item.providerId)) {
      if (item.providerId === "native-archived-conversations") { setArchiveOpenIntent({ conversationId: item.objectId, token: Date.now() }); navigateTo("archived"); }
      else { setPreferredAiConversationId(item.objectId); navigateTo("ai"); }
      return;
    }
    const local = item.localTarget;
    if (local) {
      if (item.objectType === "file" && local.localFileId) {
        dispatchWorkspace({ type: "workspaceSelected", workspaceId: local.workspaceId });
        setPendingFileTarget(local.localFileId);
        exitProject("workspace");
        return;
      }
      if (!local.projectId) {
        setMessage("该搜索结果没有关联的项目，无法进入项目详情。", "error");
        return;
      }
      openProjectDetail(local.projectId, { phaseId: local.phaseId });
      return;
    }
    const deepLink = item.deepLink;
    if (!deepLink || deepLink.kind !== "application" || deepLink.objectId !== item.objectId) {
      setMessage("该搜索结果没有可用的打开位置。", "error");
      return;
    }
    setApplicationSearchOpenIntent({
      token: randomUuid(),
      kind: "application",
      applicationId: deepLink.applicationId,
      objectType: item.objectType,
      objectId: item.objectId,
      target: deepLink.target,
    });
    navigateTo("applications");
  };

  useEffect(() => {
    setApplicationSearchOpenIntent(null);
  }, [localProfile?.profileId]);

  const [workReuse, setWorkReuse] = useState<{ kind: "skill" | "automation"; intent: WorkReuseIntent } | null>(null);
  useEffect(() => { setWorkReuse(null); }, [localProfile?.profileId]);

  const openAi = (request?: { title: string; conversationType: AiConversationType; mode?: AiConversationMode; context?: AiContext; projectIds?: string[]; selectedSkillId?: string; prefill?: string }) => {
    if (!selectedWorkspace) { setMessage("请先连接并选择本地工作区，再启动 AI 协作。", "error"); return; }
    setPreferredAiConversationId(null);
    if (request) setAiLaunchRequest({ token: Date.now(), title: request.title, conversationType: request.conversationType, mode: request.mode ?? "discuss", context: request.context ?? workspaceContext(selectedWorkspace, selectedFile, selectedProjectIdentity), projectIds: request.projectIds, selectedSkillId: request.selectedSkillId, prefill: request.prefill });
    navigateTo("ai");
  };

  const openSkillConversation = (skillId: string, skillName: string, mode: "try" | "create") => {
    if (!selectedWorkspace) {
      setMessage("请先连接并选择本地工作区，再进入对话使用技能。", "error");
      return;
    }
    openAi({
      title: mode === "create" ? (skillId === "system.capability-creator" ? "创建应用" : "创建技能") : `试用 ${skillName}`,
      conversationType: "direct",
      context: workspaceContext(selectedWorkspace, null, selectedProjectIdentity),
      projectIds: selectedProjectIdentity ? [selectedProjectIdentity.projectId] : undefined,
      selectedSkillId: skillId,
      prefill: mode === "create"
        ? skillId === "system.capability-creator" ? "请帮我创建一个个人应用。我希望它完成：\n" : "请帮我创建一个可复用技能。先询问我的目标、适用场景、输入输出和关键约束，信息充分后完成技能。"
        : `请使用「${skillName}」完成以下任务：\n`,
    });
  };

  const openFileAi = (file: WorkspaceFileItem) => {
    if (!selectedWorkspace) return;
    if (!aiProjectCatalogReady) { setMessage("项目列表正在读取，请稍后再试。", "error"); return; }
    const projects = aiProjects.filter((project) => project.workspaceIds.includes(selectedWorkspace.workspaceId) && project.primaryWorkspaceId && (project.primaryWorkspaceRoot || project.primaryTeamSpaceId));
    if (!projects.length) { setMessage("当前文件没有可用于对话的关联项目。", "error"); return; }
    const project = projects.length === 1 ? projects[0] : null;
    openAi({
      title: `讨论 ${file.relativePath}`,
      conversationType: "file",
      context: workspaceContext(selectedWorkspace, file, project),
      projectIds: projects.map((item) => item.projectId),
    });
  };

  const openConversationPage = () => {
    setAiLaunchRequest(null);
    setPreferredAiConversationId(null);
    navigateTo("ai");
  };

  const isAiHub = page === "ai" || page === "automations";
  const personalPage = page === "settings" || page === "agents" || page === "updates" || page === "diagnostics" || page === "usage" || page === "tools" || page === "data" || page === "notifications" || page === "disk-access" || page === "mywork" || page === "portfolio" || page === "search" || page === "skills" || page === "archived" || page === "applications" || isAiHub;
  const projectShellActive = projectDetail !== null && (page === "progress" || page === "workspace");
  // 侧栏高亮跟着用户刚点的那一项走。此前在项目里点「文件」会把高亮留在「项目」上，
  // 点了像没生效。项目推进没有自己的一级入口，仍然由「项目」代表，侧栏不会空着。
  const navItemActive = (id: Page) => id === "portfolio"
    ? page === "portfolio" || (projectDetail !== null && page === "progress")
    : page === id;
  const hideExplorer = personalPage || page === "progress";
  const pageTitle: Record<Page, string> = {
    mywork: "工作台 · 跨项目",
    portfolio: "项目",
    search: "全局搜索",
    automations: "自动化",
    skills: "技能",
    workspace: projectDetail ? `${projectDetail.displayName} · 项目文件` : selectedWorkspace?.displayName ?? "本地工作区",
    progress: projectDetail ? `${projectDetail.displayName} · 项目推进` : `${selectedWorkspace?.displayName ?? "本地工作区"} · 项目推进`,
    ai: "AI 工作台 · 对话",
    settings: "设置",
    agents: "设置 · 模型设置",
    archived: "设置 · 已归档会话",
    updates: "设置 · 软件更新",
    diagnostics: "设置 · 诊断",
    usage: "设置 · 使用量",
    tools: "设置 · 工具连接",
    data: "设置 · 数据",
    notifications: "设置 · 通知",
    "disk-access": "设置 · 磁盘访问",
    applications: "应用",
  };

  // AI 页的右下角是发送按钮，浮层提示默认停在那里会把主动作盖住；标记出这一页，
  // 由样式把提示改挂到右上角——模式选择器挪进输入框后，那块地方正好空着。
  /*
   * AI 浏览器占根网格最后一列。收起后列宽归零，那一列就不存在——它可能开很久，盖住内容
   * 等于这段时间工作台不能用。收起状态随浏览器停止而复位，下一轮重新出现。
   */
  const aiBrowserState = useAiBrowserState();
  const [aiBrowserCollapsed, setAiBrowserCollapsed] = useState(true);
  /*
   * 浏览器归属发起它的那次对话：切到别的对话或别的页面都不呈现，切回来还在。
   * 对话由对话页选定，这里只记住它现在是哪一个。
   */
  const [aiBrowserConversationId, setAiBrowserConversationId] = useState<string | null>(null);
  // 当前对话里出现标签时把面板推出来：员工要看得见 AI 在做什么，也要能随时接管
  const aiBrowserHasTabs = aiBrowserState.tabs.some(
    (tab) => tab.conversationId !== null && tab.conversationId === aiBrowserConversationId);
  useEffect(() => {
    if (aiBrowserHasTabs) setAiBrowserCollapsed(false);
  }, [aiBrowserHasTabs]);
  // 员工拖出来的宽度。存一份，下次打开还是这么宽
  const [aiBrowserWidth, setAiBrowserWidth] = useState(readAiBrowserWidth);
  const applyAiBrowserWidth = useCallback((width: number, persist: boolean) => {
    const next = clampAiBrowserWidth(width);
    setAiBrowserWidth(next);
    if (persist) { try { window.localStorage.setItem(aiBrowserWidthStorageKey, String(next)); } catch { /* 存不下不影响这次拖动 */ } }
  }, []);
  /*
   * 对话页自己知道它最少要多宽——侧栏能拖也能收起，这个数只有它算得准。
   * 主界面拿这个数决定浏览器让到什么程度，不在两处各写一份常量。
   */
  /*
   * 还没报上来时为空。空与"恰好等于聊天区下限"必须分得开：自动拉宽要等这个数落定才能算，
   * 拿默认值去算会少算一整条侧栏，窗口拉到一半。
   */
  const [reportedConversationMinWidth, setAiConversationMinWidth] = useState<number | null>(null);
  const aiConversationMinWidth = reportedConversationMinWidth ?? minChatStageWidth;
  const viewportWidth = useViewportWidth();
  // 活动栏宽度跟着主导航走。这里传实际值，让宽公式不再假定它永远是收起的那一档
  const aiActivityBarWidth = activityBarWidthFor(navigationExpanded);
  const aiBrowserFit = fitAiBrowser({
    viewportWidth, activityBarWidth: aiActivityBarWidth,
    conversationMinWidth: aiConversationMinWidth, requestedWidth: aiBrowserWidth });
  /*
   * 真正可见的条件：员工没收起、停在对话页、当前对话确实开着标签、且窗口放得下。
   * 列宽与面板用同一个判定——两处各算一份，迟早出现空着一列却什么都不显示。
   * 放不下时只是不呈现，收起标记不动：窗口拉宽了就自己回来，不用员工再点一次。
   */
  const [activeApplicationId, setActiveApplicationId] = useState<string | null>(null);
  const applicationBrowserScope = activeApplicationId ? [...aiBrowserState.tabs].reverse().find(tab => tab.conversationId?.startsWith(`application:${activeApplicationId}:`))?.conversationId ?? null : null;
  useEffect(() => {
    if (applicationBrowserScope) setAiBrowserCollapsed(false);
  }, [applicationBrowserScope]);
  const aiBrowserConversation = page === "ai" ? aiBrowserConversationId : page === "applications" ? applicationBrowserScope : null;
  /*
   * "员工要它出来"与"现在放得下"分成两个判定。合成一个的话，窗口太窄时连"员工想展开"
   * 这件事都不成立，自动拉宽就无从触发——正是窗口窄的时候最需要拉。
   */
  const aiBrowserWanted = !aiBrowserCollapsed
    && aiBrowserConversation !== null
    && aiBrowserState.tabs.some((tab) => tab.conversationId === aiBrowserConversation);
  const aiBrowserOpen = aiBrowserWanted && aiBrowserFit.fits;
  /*
   * 窗口不够宽就把窗口拉宽，而不是让浏览器一路退让。返回宿主实际给到的内容宽度：屏幕本来
   * 就不够宽，或窗口已最大化、全屏而拉不动时，它小于要的数，那时仍旧由让宽逻辑收场。
   */
  const widenWindowForAiBrowser = useCallback(async (target: number): Promise<number> => {
    try {
      const result = await invoke<{ width: number }>("plugin:window|ensure_content_width", { width: target });
      return Number.isFinite(result?.width) ? result.width : window.innerWidth;
    } catch (error) {
      // 拉不动是可以接受的，拉窗口这件事整个失灵不是——不说出来，员工只会看到浏览器莫名偏窄
      setMessage(`窗口拉宽失败，浏览器只能用剩下的宽度：${String(error)}`, "error");
      return window.innerWidth;
    }
  }, [setMessage]);
  /*
   * 展开那一下拉一次，之后不再自动改尺寸：员工自己拖浏览器或拖侧栏时窗口跟着跳，比窄一点
   * 更难用。收起后复位，下次展开再拉一次。对话页的下限还没报上来就先不动手——那一刻用默认
   * 值算会少一整条侧栏，窗口只拉到一半，比不拉更难看。
   */
  const aiBrowserWidenedRef = useRef(false);
  useEffect(() => {
    if (!aiBrowserWanted) { aiBrowserWidenedRef.current = false; return; }
    if (aiBrowserWidenedRef.current || reportedConversationMinWidth === null) return;
    aiBrowserWidenedRef.current = true;
    const target = desiredViewportForAiBrowser({
      activityBarWidth: aiActivityBarWidth,
      conversationMinWidth: reportedConversationMinWidth,
      requestedWidth: aiBrowserWidth });
    if (target <= window.innerWidth) return;
    void widenWindowForAiBrowser(target);
  }, [aiActivityBarWidth, aiBrowserWanted, aiBrowserWidth, reportedConversationMinWidth,
    widenWindowForAiBrowser]);

  const standaloneWindowControls = <div className="title-strip standalone-title-strip" data-platform={hostPlatform ?? undefined} data-window-drag-region>
    <DesktopUpdateControl platformSignal={eventSignals.clientUpdateSignal}/>
    {hostPlatform === "windows" ? <WindowControls maximized={windowMaximized}/> : null}
  </div>;
  if (status?.personalDataState === "applying" || status?.personalDataState === "restart_required") {
    return <main className="standalone-screen">{standaloneWindowControls}<PersonalRestoreStatus applying={status.personalDataState === "applying"}/></main>;
  }
  if (status?.runtimeInitializationError && /personal_(?:data_)?maintenance_/.test(status.runtimeInitializationError)) {
    return <main className="standalone-screen">{standaloneWindowControls}<section><h1>资料维护尚未结束</h1><PersonalMaintenanceRecovery onRecovered={restartAppServer}/></section></main>;
  }
  if (status?.phase === "failed" || status?.runtimeInitializationError) {
    return <main className="standalone-screen">{standaloneWindowControls}<Failure status={{ ...status, errorReason: status.runtimeInitializationError ?? status.errorReason }} title={status.runtimeInitializationError ? "历史会话恢复失败" : undefined} onRestart={restartAppServer} windows={hostPlatform === "windows"}/></main>;
  }
  if (!localProfile) {
    return <main className="standalone-screen">{standaloneWindowControls}<section aria-label="本地资料">
      <h1>{localProfileError ? "无法打开本地资料" : "正在打开工作台"}</h1>
      {localProfileError ? <><p role="alert">{localProfileError}</p><button onClick={() => void refreshLocalProfile()} type="button">重试</button>{hostPlatform === "windows" ? <StartupPermissions onRecovered={() => void refreshLocalProfile()}/> : null}</> : null}
    </section></main>;
  }
  const profile: LocalUserProfile = { displayName: localProfile.displayName, profileId: localProfile.profileId };

  return <main className={`shell${personalPage ? " my-work-shell" : ""}${hideExplorer ? " hide-explorer" : ""}${navigationExpanded ? " navigation-expanded" : ""}${page === "ai" ? " ai-chat-shell" : ""}${resizingExplorer ? " resizing-explorer" : ""}`} data-platform={hostPlatform ?? undefined} style={{
    "--explorer-sidebar-width": `${explorerWidth}px`,
    /*
     * 宽度让着内容区，且不设下限。对话页的网格有硬下限——侧栏 304 加舞台下限 420，再加
     * 活动栏 48，合计 772；低于这个宽度对话会被切掉一截。此前给浏览器列留了 320 的保底，
     * 结果窄窗口下正是那条保底把对话挤破了：宁可浏览器窄一点，也不能让对话被切。
     */
    "--ai-browser-width": aiBrowserOpen ? `${aiBrowserFit.width}px` : "0px",
  } as CSSProperties}>
    {/* macOS 用系统标题栏叠加窗口内容，标题文字隐藏，红绿灯留给系统绘制，这条自绘窄条
        只接管红绿灯右侧的空间：主导航收起按钮，剩下的整片都是窗口拖动区。Windows 关闭了
        系统装饰，同一条窄条改为承载自绘的最小化/最大化/关闭按钮。 */}
    <div className="title-strip" data-window-drag-region>
      <button aria-expanded={navigationExpanded} aria-label={navigationExpanded ? "收起主导航" : "展开主导航"} className="activity-nav-toggle" onClick={() => setNavigationExpanded((current) => !current)} title={navigationExpanded ? "收起主导航" : "展开主导航"} type="button"><PanelToggleIcon expanded={navigationExpanded}/></button>
      <DesktopUpdateControl platformSignal={eventSignals.clientUpdateSignal}/>
      {hostPlatform === "windows" ? <WindowControls maximized={windowMaximized}/> : null}
    </div>

    <aside className="activity-bar">
      <header className="activity-nav-header"><strong className="activity-brand"><img alt="" aria-hidden="true" className="activity-brand-mark" src={cilaiSymbolUrl}/><span>词来</span></strong><button aria-current={page === "search" ? "page" : undefined} aria-label="全局搜索" className={`activity-nav-search${page === "search" ? " active" : ""}`} onClick={() => navigateTo("search")} title={platformUi ? `全局搜索 (${platformUi.globalSearchShortcut})` : "全局搜索"} type="button"><GlobalSearchIcon/></button></header>
      <nav aria-label="主导航" className="activity-nav">{navItems.map((item) => { const active = navItemActive(item.id); return <HoverTip enabled={!navigationExpanded} key={item.id} label={item.label}>{(tip) => <button aria-current={active ? "page" : undefined} aria-label={item.label} className={active ? "active" : ""} onClick={() => item.id === "ai" ? openConversationPage() : navigateTo(item.id)} onPointerDown={tip.onPointerDown} onPointerEnter={tip.onPointerEnter} onPointerLeave={tip.onPointerLeave} ref={tip.ref as never} type="button">{item.icon}<span>{item.label}</span></button>}</HoverTip>; })}</nav>
      <UserMenu expanded={navigationExpanded} profile={profile} theme={theme} onThemeChange={setTheme} onOpenSettings={() => navigateTo("settings")}/>
    </aside>
    <aside className="explorer-sidebar">
      <header className="explorer-header"><strong>{projectShellActive ? "项目文件" : "文件"}</strong><div className="explorer-actions"><button aria-label={showHiddenItems ? "隐藏隐藏文件" : "显示隐藏文件"} aria-pressed={showHiddenItems} className={showHiddenItems ? "active" : ""} disabled={!selectedWorkspace} onClick={() => setShowHiddenItems((current) => !current)} title={showHiddenItems ? "隐藏以 . 开头的文件和文件夹" : "显示以 . 开头的文件和文件夹"} type="button">{showHiddenItems ? <EyeOffIcon/> : <EyeIcon/>}</button><button aria-label="重新扫描" disabled={!selectedWorkspace || workspaceOperationBusy || selectedWorkspace.workspaceKind !== "local"} onClick={() => void scan()} title="重新扫描" type="button"><RefreshIcon/></button><button aria-label="新建文件夹" disabled={!selectedWorkspace || workspaceOperationBusy || selectedWorkspace.workspaceKind !== "local"} onClick={() => {
        if (selectedWorkspace) setFolderCreation({ workspace: selectedWorkspace, parentRelativePath: selectedDirectory?.workspaceId === selectedWorkspace.workspaceId ? selectedDirectory.path : currentDirectory(selectedFile) });
      }} title="新建文件夹" type="button"><AddFolderIcon/></button></div></header>
      <div className="explorer-footer explorer-space-picker" ref={workspaceSwitcher}>
        {selectedWorkspace ? <>
          <button aria-expanded={workspaceSwitcherOpen} aria-haspopup="dialog" className="workspace-switcher-trigger" disabled={workspaceOperationBusy} onClick={() => setWorkspaceSwitcherOpen((current) => !current)} type="button"><WorkspaceIcon/><span>{selectedProjectIdentity ? <strong>{selectedProjectIdentity.displayName}</strong> : null}<small>{selectedWorkspace.workspaceKind === "local" ? "本地" : "旧云端来源"} · {selectedWorkspace.displayName}</small></span><SwitcherIcon/></button>
          {workspaceSwitcherOpen ? <WorkspaceSwitcher
            addWorkspaceDisabled={workspaceOperationBusy || !aiProjectCatalogReady}
            workspaces={workspaces}
            projectIndex={workspaceProjects}
            selectedWorkspaceId={selectedWorkspace.workspaceId}
            onAddWorkspace={() => {
              setWorkspaceSwitcherOpen(false);
              setWorkspaceAddOpen(true);
            }}
            onSelect={selectWorkspace}/> : null}
        </> : <button disabled={workspaceOperationBusy} onClick={() => void createWorkspace()} type="button"><AddFolderIcon/><span>{connectingWorkspace ? "正在选择…" : "连接工作区"}</span></button>}
      </div>
      {selectedWorkspace ? <div className="explorer-file-search"><input aria-label="搜索已索引文件" placeholder="搜索已索引文件" value={fileQuery} onChange={(event) => setFileQuery(event.target.value)}/>{fileQuery ? <button aria-label="清除文件搜索" onClick={() => setFileQuery("")} type="button">清除</button> : null}</div> : null}
      {unindexedFileDownloading ? <div className="workspace-index-line" role="status"><span className="workspace-index-text">正在下载</span><button type="button" onClick={() => unindexedFileRead.current?.abort()}>取消</button></div> : null}
      {selectedWorkspace ? <><WorkspaceIndexLine label={indexProgressLabel(workspaceIndex.status) ?? (workspaceIndex.pollError ? `无法读取索引进度：${workspaceIndex.pollError}` : null)} failed={workspaceIndex.status?.phase === "failed" || workspaceIndex.pollError !== null} active={isIndexActive(workspaceIndex.status)} onRetry={() => void startWorkspaceIndex(selectedWorkspace.workspaceId).catch((error) => setMessage(`重新建立索引没有启动：${describeServerError(error)}`, "error"))}/>{fileQuery.trim() ? <WorkspaceFileSearch files={displayedFiles} query={fileQuery} loadState={loadState} error={loadError} onRetry={() => void loadFiles(selectedWorkspace.workspaceId)} onFile={selectFile}/> : selectedWorkspace.workspaceKind === "local" ? <LazyFileTree key={selectedWorkspace.workspaceId} workspaceId={selectedWorkspace.workspaceId} workspaceKind={selectedWorkspace.workspaceKind} indexedFiles={displayedFiles} showHidden={showHiddenItems} selectedFileId={selectedDirectory?.workspaceId === selectedWorkspace.workspaceId ? null : selectedFile?.localFileId ?? null} selectedDirectoryPath={selectedDirectory?.workspaceId === selectedWorkspace.workspaceId ? selectedDirectory.path : null} pendingPath={indexingPath} refreshToken={layeredTreeRefresh} onDirectory={(path) => { unindexedFileRead.current?.abort(); setSelectedDirectory({ workspaceId: selectedWorkspace.workspaceId, path }); }} onContext={openFileContextMenu} onFile={selectFile} onUnindexedFile={(path) => void openUnindexedFile(selectedWorkspace.workspaceId, path)}/> : <p className="explorer-load-state" role="alert">旧云端来源不可用</p>}</> : <div className="explorer-empty"><Folder/><strong>尚未连接目录</strong><span>连接后在此浏览文件结构</span></div>}

      {page === "workspace" ? <div
        aria-label="调整文件列表宽度"
        aria-orientation="vertical"
        aria-valuemax={maxExplorerWidth}
        aria-valuemin={minExplorerWidth}
        aria-valuenow={explorerWidth}
        className="explorer-resizer"
        onDoubleClick={() => applyExplorerWidth(defaultExplorerWidth, true)}
        onKeyDown={resizeExplorerWithKeyboard}
        onPointerDown={startExplorerResize}
        role="separator"
        tabIndex={0}
        title="拖动调整文件列表宽度；双击恢复默认宽度"/> : null}
    </aside>
    <section className="content app-content">
      {(!personalPage && page !== "progress" && page !== "workspace") ? <header className="app-header"><div>{projectShellActive ? <button aria-label="返回项目列表" className="project-back" onClick={() => exitProject()} title="返回项目列表" type="button"><BackArrowIcon/>返回</button> : null}<h1>{pageTitle[page]}</h1></div></header> : null}
      {editTarget ? <ProjectEditDialog
      connectingWorkspace={workspaceOperationBusy}
      hostPlatform={hostPlatform}
      target={editTarget}
      workspaces={workspaces}
      onAddFolder={() => void addFolderToProject()}
      onClose={() => setEditTarget(null)}
      onMessage={setMessage}
      onRemoveFolder={setPendingFolderAction}
      onReveal={(workspace) => void revealWorkspace(workspace)}
      onSaved={(project) => {
        setEditTarget(null);
        setProjectDetail((current) => current?.projectId === project.projectId ? { ...current, displayName: project.displayName } : current);
        setAiProjectRefreshToken((value) => value + 1);
      }}/> : null}
    {folderCreation ? <CreateFolderDialog workspace={folderCreation.workspace} parentRelativePath={folderCreation.parentRelativePath} onClose={() => setFolderCreation(null)} onCreated={async (path) => {
      // 本地新建的文件夹只补录这一条，不必把整个工作区重扫一遍。
      if (folderCreation.workspace.workspaceKind === "local") await indexWorkspacePath(folderCreation.workspace.workspaceId, path);
      else throw new Error("personal_cloud_source_unavailable");
      await loadFiles(folderCreation.workspace.workspaceId);
      setLayeredTreeRefresh((value) => value + 1);
      setSelectedDirectory({ workspaceId: folderCreation.workspace.workspaceId, path });
    }}/> : null}
    {workspaceAddOpen ? <LocalWorkspaceDialog
      onAddLocal={(project) => {
        setWorkspaceAddOpen(false);
        void addFolderToProject(project);
      }}
      onClose={() => setWorkspaceAddOpen(false)}
      onCreateProject={() => {
        setWorkspaceAddOpen(false);
        setProjectCreateForFolder(true);
        setAiProjectCreateOpen(true);
      }}
      projects={aiProjects}
      target={selectedProjectIdentity ? {
        ...selectedProjectIdentity,
        workspaceIds: aiProjects.find((project) => project.projectId === selectedProjectIdentity.projectId)?.workspaceIds ?? [],
      } : undefined}/> : null}
    {aiProjectCreateOpen ? <ProjectCreatePanel
      existingProjects={projectCreateForFolder ? [] : aiProjects.map((project) => ({ projectId: project.projectId, displayName: project.displayName }))}
      onAddWorkspace={createWorkspace}
      onCancel={() => {
        setAiProjectCreateOpen(false);
        setProjectCreateForFolder(false);
      }}
      onCreated={(project, workspaceId) => {
        setAiProjectCreateOpen(false);
        setProjectCreateForFolder(false);
        setAiProjectRefreshToken((value) => value + 1);
        void loadWorkspaces();
        if (workspaceId) dispatchWorkspace({ type: "workspaceSelected", workspaceId });
      }}
      onMessage={setMessage}
      workspaces={workspaces}/> : null}
    {/* AI 浏览器的写操作逐次确认。与页面无关，因此挂在根部，不随页面切换消失。 */}
    <BrowserApprovalSurface />
    {message && message.tone === "error" ? <AlertDialog message={message.text} title="操作未完成" onClose={() => setMessage(null)}/> : null}
    <ContextMenu state={contextMenu} onClose={() => setContextMenu(null)}/>
    {pendingFileDelete ? <ConfirmDialog
        busy={workspaceOperationBusy}
        confirmLabel={pendingFileDelete.fileType === "directory" ? "删除文件夹" : "删除文件"}
        danger
        description={<><p>将把「{pendingFileDelete.relativePath}」从磁盘上删除{pendingFileDelete.fileType === "directory" ? "，其中的全部内容一并删除" : ""}。</p><p>{platformUi ? `删除后会移到${platformUi.trashName}。` : "应用中该文件的确认记录与变更历史保留。"}</p></>}
        title={pendingFileDelete.fileType === "directory" ? "删除这个文件夹？" : "删除这个文件？"}
        onCancel={() => setPendingFileDelete(null)}
        onConfirm={() => { const target = pendingFileDelete; setPendingFileDelete(null); void deleteWorkspaceItem(target); }}/> : null}
    {pendingFolderAction ? <ConfirmDialog
        busy={workspaceOperationBusy}
        confirmLabel="移出项目"
        description={<><p>将把「{pendingFolderAction.displayName}」移出当前项目，之后它可以被加入其他项目。</p><p>应用内索引、扫描结果、变更历史以及磁盘文件都会保留，不需要重新扫描。</p></>}
        onCancel={() => setPendingFolderAction(null)}
        onConfirm={() => {
          const pending = pendingFolderAction;
          setPendingFolderAction(null);
          void removeFolderFromProject(pending);
        }}
        title="移出项目"/> : null}
      {archiveNotice ? <ArchiveNotice key={archiveNotice.id} operation={archiveNotice} onMessage={setMessage}
        onClose={() => setArchiveNotice((current) => current?.id === archiveNotice.id ? null : current)}
        onView={(conversationId) => { setArchiveOpenIntent({ conversationId, token: Date.now() }); navigateTo("archived"); }}/>: null}
      {message && message.tone === "info" ? <div className="notice" role="status"><span>{message.text}</span><button aria-label="关闭提示" onClick={() => setMessage(null)} type="button">关闭</button></div> : null}
      <ModelCatalogProvider key={localProfile.profileId} profileId={localProfile.profileId} enabled={status?.phase === "running"}>
      <WindowsSandboxProvider scopeKey={JSON.stringify([localProfile.profileId,status?.serverInstanceId])} enabled={status?.phase === "running"}>
      <ConversationSessionProvider dispatchEnabled={status?.phase === "running"} key={localProfile.profileId} hostPlatform={hostPlatform} onMessage={setMessage}>
      <ApplicationHandoffs projects={aiProjects} projectsReady={aiProjectCatalogReady} projectsError={aiProjectCatalogError} onRetryProjects={() => setAiProjectRefreshToken((value) => value + 1)} onOpen={(conversationId) => { setAiLaunchRequest(null); setPreferredAiConversationId(conversationId); navigateTo("ai"); }} />
      <Suspense fallback={page === "applications" ? <div className="page-loading" role="status">正在加载…</div> : null}>
      <ApplicationsPage onOpenModels={() => navigateTo("agents")} onActiveApplicationChange={setActiveApplicationId} browser={{ available: Boolean(applicationBrowserScope), open: aiBrowserOpen, onToggle: () => setAiBrowserCollapsed(current => !current) }} onCreate={() => openSkillConversation("system.capability-creator", "应用创建器", "create")} active={page === "applications"} key={localProfile.profileId} profileId={localProfile.profileId} onMessage={setMessage} openIntent={applicationSearchOpenIntent} onOpenIntentConsumed={(token) => setApplicationSearchOpenIntent((current) => current?.token === token ? null : current)}/>
      </Suspense>
      <RetainedPage active={page === "mywork"}><MyWorkPage recentConversations={recentConversations} recentFailure={recentConversationError ? { error: recentConversationError, retry: () => setRecentConversationRefresh((value) => value + 1) } : undefined} onStartTask={(projectId) => { setAiLaunchRequest({ token: Date.now(), title: "新建任务", conversationType: "project", mode: "discuss", context: unclassifiedConversationContext(), projectIds: projectId ? [projectId] : undefined }); setPreferredAiConversationId(null); navigateTo("ai"); }} onStartConversation={() => { setAiLaunchRequest({ token: Date.now(), title: "新对话", conversationType: "direct", mode: "discuss", context: unclassifiedConversationContext(), startFresh: true }); setPreferredAiConversationId(null); navigateTo("ai"); }} workspaces={workspaces} eventSequence={eventSignals.nextSequence + myWorkRefreshToken} onMessage={setMessage} onOpenConversation={(conversationId) => { setPreferredAiConversationId(conversationId); navigateTo("ai"); }} onOpenRun={(runId) => void openRunConversation(runId)} onOpenSource={openWorkItem}/></RetainedPage>
      <RetainedPage active={page === "portfolio"}><ProjectPortfolioPage createIntent={portfolioCreateIntent} deleteProjectId={portfolioDeleteProjectId} onEditProject={(projectId) => void openProjectEditor(projectId)} onAddWorkspace={createWorkspace} workspaces={workspaces} eventSequence={eventSignals.nextSequence} onMessage={setMessage} onOpenProgress={(_workspaceId, targetProjectId, phaseId) => openProjectDetail(targetProjectId, { phaseId })}/></RetainedPage>
      <RetainedPage active={page === "search"}><GlobalSearchPage key={localProfile.profileId} profileId={localProfile.profileId} hostPlatform={hostPlatform} onMessage={setMessage} onOpen={openSearchItem}/></RetainedPage>
      <RetainedPage active={page === "automations"}><AutomationsPage openIntent={automationOpenIntent} onOpenIntentConsumed={(token) => setAutomationOpenIntent(current => current?.token === token ? null : current)} reuseIntent={workReuse?.kind === "automation" ? workReuse.intent : null} onReuseConsumed={() => setWorkReuse(null)} projects={aiProjects} project={selectedProjectIdentity} workspace={selectedWorkspace} onMessage={setMessage} onOpenRun={(runId) => void openRunConversation(runId)}/></RetainedPage>
      <RetainedPage active={page === "skills"}><SkillsPage reuseIntent={workReuse?.kind === "skill" ? workReuse.intent : null} onReuseConsumed={() => setWorkReuse(null)} key={localProfile.profileId} openSkillId={openSkillId} onMessage={setMessage} onOpenConversation={openSkillConversation}/></RetainedPage>
      <RetainedPage active={page === "workspace"}><WorkspacePage hostPlatform={hostPlatform} loadError={loadError} onRetryLoad={() => void loadWorkspaces()} detailIntent={fileDetailIntent} diffIntent={fileDiffIntent} workspaces={workspaces} selectedWorkspace={selectedWorkspace} files={files} selectedFile={selectedFile} onFile={selectFile} revision={workspaceRevision} conflicts={conflicts} createWorkspace={async () => { await createWorkspace(); }} connectingWorkspace={connectingWorkspace} loadState={loadState} onMessage={setMessage} onOpenAi={() => { if (selectedFile) openFileAi(selectedFile); }} onConfirmationChanged={() => selectedWorkspace ? loadFiles(selectedWorkspace.workspaceId) : undefined} deliverableBinding={selectedFile ? fileDeliverableLookup.get(selectedFile.localFileId) ?? null : null}/></RetainedPage>
      <RetainedPage active={page === "progress"}><ProjectProgressPage operatorName={profile.displayName} onOpenRun={(runId) => void openRunConversation(runId)} hostPlatform={hostPlatform} workspace={selectedWorkspace} files={files} eventSequence={eventSignals.nextSequence} initialProjectId={projectDetail?.projectId ?? null} initialPhaseId={projectDetail?.selectedPhaseId ?? null} onMessage={setMessage} onOpenAi={(request) => openAi(request)} onOpenConversation={(conversationId) => { setPreferredAiConversationId(conversationId); navigateTo("ai"); }} onOpenFile={openWorkspaceFile} onExitProject={() => exitProject()} onPhaseSelected={(phaseId, phaseName) => setProjectDetail((current) => current && (current.selectedPhaseId !== phaseId || current.selectedPhaseName !== phaseName) ? { ...current, selectedPhaseId: phaseId, selectedPhaseName: phaseName } : current)}/></RetainedPage>
      <RetainedPage active={page === "ai"}><AiWorkbenchPage onReuseWork={(kind, intent) => { setWorkReuse({ kind, intent }); if (kind === "skill") setOpenSkillId(null); navigateTo(kind === "skill" ? "skills" : "automations"); }} key={localProfile.profileId} onOpenSkill={(id) => { setOpenSkillId(id); navigateTo("skills"); }} aiBrowser={{ open: aiBrowserOpen, state: aiBrowserState, conversationId: aiBrowserConversationId, onToggle: () => {
            const intent = aiBrowserToggleIntent({
              open: aiBrowserOpen, conversationId: aiBrowserConversationId,
              hasTabs: aiBrowserHasTabs, fits: aiBrowserFit.fits });
            if (intent.action === "collapse") { setAiBrowserCollapsed(true); return; }
            if (intent.action === "needConversation") { setMessage("先选一个对话，浏览器要归属到某次对话", "error"); return; }
            const expandAiBrowser = (openTab: boolean) => {
              setAiBrowserCollapsed(false);
              if (!openTab) return;
              void invoke("browser_tab_open", { conversationId: aiBrowserConversationId })
                .catch((error: unknown) => setMessage(`打开浏览器失败：${String(error)}`, "error"));
            };
            if (intent.action === "tooNarrow") {
              /*
               * 窄不再是拒绝的理由：先把窗口拉宽，拉够了就照常展开。拉不动——屏幕本来就窄，
               * 或者窗口已最大化、全屏——才说出来，此时不说按钮就成了点了没反应的哑键。
               */
              const need = minViewportForAiBrowser(aiConversationMinWidth, aiActivityBarWidth);
              void widenWindowForAiBrowser(desiredViewportForAiBrowser({
                activityBarWidth: aiActivityBarWidth,
                conversationMinWidth: aiConversationMinWidth,
                requestedWidth: aiBrowserWidth,
              })).then((width) => {
                if (width < need) {
                  setMessage(`窗口再宽一些才放得下浏览器，至少 ${need} 像素`, "error");
                  return;
                }
                expandAiBrowser(!aiBrowserHasTabs);
              });
              return;
            }
            expandAiBrowser(intent.openTab);
          }, onConversationChange: setAiBrowserConversationId, onMinWidthChange: setAiConversationMinWidth }} workspaces={workspaces} workspace={selectedWorkspace} projects={aiProjects} files={files} selectedFile={selectedFile} launchRequest={aiLaunchRequest} preferredConversationId={preferredAiConversationId} onPreferredConversationConsumed={(id) => setPreferredAiConversationId((current) => current === id ? null : current)} profileId={localProfile.profileId} hostPlatform={hostPlatform} onMessage={setMessage} onOpenAgents={() => navigateTo("agents")} onOpenFile={openWorkspaceFile} onOpenWorkspacePath={openWorkspacePath} canCreateProject={aiProjectCatalogReady} onCreateProject={() => { setProjectCreateForFolder(false); setAiProjectCreateOpen(true); }}
          onLaunchConsumed={(token) => setAiLaunchRequest((current) => current?.token === token ? null : current)} projectCreationOpen={aiProjectCreateOpen} onSelectProject={(projectId) => { const project = aiProjects.find((item) => item.projectId === projectId); if (project && (!selectedWorkspace || !project.workspaceIds.includes(selectedWorkspace.workspaceId))) dispatchWorkspace({ type: "workspaceSelected", workspaceId: project.primaryWorkspaceId }); }} projectIndex={workspaceProjectIndex}/></RetainedPage>
      <RetainedPage active={(page === "settings" || page === "agents" || page === "archived" || page === "updates" || page === "diagnostics" || page === "usage" || page === "tools" || page === "data" || page === "disk-access" || page === "notifications")}><SettingsPage key={localProfile.profileId} profileId={localProfile.profileId} section={settingsSectionFor(page)} onSectionChange={(section) => { if (section === "archived") setArchiveOpenIntent(null); navigateTo(section === "memory" ? "settings" : section); }} workspace={selectedWorkspace} hostPlatform={hostPlatform} onMessage={setMessage} projects={aiProjects} archiveOpenIntent={archiveOpenIntent}/></RetainedPage>
      </ConversationSessionProvider>
      </WindowsSandboxProvider>
      </ModelCatalogProvider>
    </section>
    {/*
      AI 浏览器占根网格最后一列，因此必须是根元素的直接子元素——放在内容区里的话网格定位
      对它不起作用，它会挤在内容列里。旁边那些浮层用的是传送门，放哪儿都行，这个不行。
    */}
    {/*
      面板始终挂载，收起只是隐藏。卸载会连同标签的内容元素一起移除——AI 正在浏览的页面
      会被就此杀掉，员工收一下面板就等于打断了它。
    */}
    <AiBrowserPanel conversationId={aiBrowserConversation} onClose={() => setAiBrowserCollapsed(true)} onWidthChange={applyAiBrowserWidth} open={aiBrowserOpen} state={aiBrowserState} width={aiBrowserWidth}/>
  </main>;
}


function appServerFailureDetail(status: AppServerStatus) {
  const details = [...status.stderrTail].reverse()
    .map((line) => line.trim().replace(/^App Server failure:\s*/, ""))
    .filter(Boolean);
  const stderr = details.find((line) => line !== "initialize_failed: App Server connection closed") ?? details[0];
  return stderr
    ?? status.errorReason
    ?? "本地服务没有返回启动失败原因";
}

function Failure({ status, onRestart, title = "本地服务启动失败", windows }: { status: AppServerStatus; onRestart: () => void; title?: string; windows?: boolean }) {
  const detail = appServerFailureDetail(status);
  const nativeHistoryError = /\bnative_history_[a-z_]+\b|codex_thread_request_failed:thread\/.*thread not loaded:/.test(detail);
  return <div className="failure">
    <h2>{title}</h2>
    <p>{detail}</p>
    {/\bnative_history_content_mismatch\b/.test(detail) ? <p>历史消息内容校验未通过，恢复已停止。请导出错误日志，保留当前数据以便核查。</p> : null}
    {windows && !nativeHistoryError ? <StartupPermissions/> : null}
    {status.lastExitCode === null ? null : <small>退出码 {status.lastExitCode}</small>}
    <button onClick={onRestart} type="button">重新启动</button>
    <ExportDiagnosticsButton/>
  </div>;
}
/** 同一个目录可能一次带结尾斜杠一次不带，比较前先抹平。 */
function sameLocalRoot(left: string, right: string) { return left.replace(/\/+$/, "") === right.replace(/\/+$/, ""); }
/** 文件树顶部的索引进度行。空闲时不占位置；失败时给出原因和重试。 */
function WorkspaceIndexLine(props: { label: string | null; active: boolean; failed: boolean; onRetry: () => void }) {
  if (!props.label) return null;
  return <div aria-live="polite" className="workspace-index-line" data-state={props.failed ? "failed" : props.active ? "active" : "idle"} role="status">
    {props.active ? <span aria-hidden="true" className="workspace-index-pulse"/> : null}
    <span className="workspace-index-text" title={props.label}>{props.label}</span>
    {props.failed ? <button onClick={props.onRetry} type="button">重试</button> : null}
  </div>;
}

function isHiddenPath(path: string) { return path.split("/").some((segment) => segment.startsWith(".") && segment !== "." && segment !== ".."); }

// Guarded so tests can import App without the module mounting itself; in the real app
// index.html always provides #root.
const rootElement = document.getElementById("root");
if (rootElement) createRoot(rootElement).render(<StrictMode><App/></StrictMode>);
