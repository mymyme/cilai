import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { WorkspaceFileItem, WorkspaceSummary } from "./generated/protocol";
import { fileNameFromRelativePath, WorkspacePage, WorkspaceSwitcher } from "./workspace-page";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({
  invoke,
}));
vi.mock("./app-server-rpc", async (importOriginal) => ({ ...await importOriginal<typeof import("./app-server-rpc")>(), rpc: async (method: string, params: object) => method === "file/hydrate" ? { state: "ready" } : invoke("app_server_request", { method, params }) }));

afterEach(() => { cleanup(); vi.clearAllMocks(); });

/**
 * 文件树和工作区切换器是从千行的应用外壳里整体搬出来的，搬之前它们一行覆盖都没有：
 * 类型检查只能保证符号接得上，保证不了渲染出来还是原来那个样子。
 */

const file = (over: Partial<WorkspaceFileItem>) => ({
  localFileId: "f1",
  relativePath: "01合同/合同文件.md",
  fileType: "file",
  presenceState: "present",
  localChangeState: "clean",
  confirmationState: "not_required",
  conflictState: "none",
  ...over,
} as WorkspaceFileItem);

const workspace = (over: Partial<WorkspaceSummary>) => ({
  workspaceId: "ws-1",
  workspaceKind: "local",
  displayName: "示例工作区",
  localRoot: "/Users/someone/项目",
  localStatus: "ready",
  platformBound: false,
  ...over,
} as WorkspaceSummary);

const projectIndex = new Map([
  ["ws-1", [{ projectId: "project-1", displayName: "示例项目" }]],
  ["ws-2", [{ projectId: "project-2", displayName: "另一个项目" }]],
]);

describe("文件树", () => {
  it("文件页标题从相对路径中取得当前文件名称", () => {
    const relativePath = "03策划/03产品规划/词来产品详细介绍.html";
    expect(fileNameFromRelativePath(relativePath)).toBe("词来产品详细介绍.html");
    const selectedWorkspace = workspace({});
    const selectedFile = file({ relativePath });
    const rendered = render(<WorkspacePage
      conflicts={[]}
      connectingWorkspace={false}
      createWorkspace={() => Promise.resolve()}
      deliverableBinding={null}
      detailIntent={0}
      diffIntent={0}
      files={[selectedFile]}
      hostPlatform="macos"
      loadError={null}
      loadState="ready"
      onFile={() => {}}
      onMessage={() => {}}
      onOpenAi={() => {}}
      onRetryLoad={() => {}}
      revision={1}
      selectedFile={selectedFile}
      selectedWorkspace={selectedWorkspace}
      workspaces={[selectedWorkspace]}/>
    );
    const title = rendered.container.querySelector(".document-view-context strong");
    expect(title?.textContent).toBe("词来产品详细介绍.html");
    expect(title?.getAttribute("title")).toBe(relativePath);
  });

  it("文件预览不再显示本地同步确认操作", async () => {
    const body = "# 标题";
    invoke.mockImplementation((command: string) => command === "preview_open"
      ? Promise.resolve({ source: "local", availability: "ready", format: "markdown", relativePath: "项目说明.md", contentRevision: "rev1", ticketId: "ticket-1", byteLength: body.length, truncated: false })
      : command === "text_resource_read"
        ? Promise.resolve({ contentRevision: "rev1", offset: 0, text: body, nextOffset: body.length, endOfResource: true })
        : Promise.resolve());
    const selectedWorkspace = workspace({});
    const selectedFile = file({
      relativePath: "项目说明.md",
      confirmationState: "pending",
      checksumAlgorithm: "sha256",
      checksumValue: "abc",
      baselineVersionId: null,
      stateRevision: 1,
    });
    const rendered = render(<WorkspacePage
      conflicts={[]}
      connectingWorkspace={false}
      createWorkspace={() => Promise.resolve()}
      deliverableBinding={null}
      detailIntent={0}
      diffIntent={0}
      files={[selectedFile]}
      hostPlatform="macos"
      loadError={null}
      loadState="ready"
      onFile={() => {}}
      onMessage={() => {}}
      onOpenAi={() => {}}
      onRetryLoad={() => {}}
      revision={1}
      selectedFile={selectedFile}
      selectedWorkspace={selectedWorkspace}
      workspaces={[selectedWorkspace]}/>);

    expect(await screen.findByRole("heading", { name: "标题" })).not.toBeNull();
    const toggle = await screen.findByRole("button", { name: "切换到源码" });
    expect(toggle.parentElement?.classList.contains("document-toolbar-actions")).toBe(true);
    expect(screen.queryByRole("button", { name: "待确认" })).toBeNull();
    expect(screen.getByRole("button", { name: "AI 协作" }).nextElementSibling?.classList.contains("toolbar-more")).toBe(true);
    expect(rendered.container.querySelector(".preview-view-tabs")).toBeNull();

    fireEvent.click(toggle);
    const previewToggle = screen.getByRole("button", { name: "切换到预览" });
    expect(previewToggle.getAttribute("aria-pressed")).toBeNull();
    expect(previewToggle.classList.contains("active")).toBe(false);
    expect(screen.queryByRole("heading", { name: "标题" })).toBeNull();
    expect(screen.getByText(/# 标题/)).not.toBeNull();
  });

  it("本地文件通过更多菜单确认内容，不提供同步提交", async () => {
    const body = "# 新文档";
    const emptyRevision = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    invoke.mockImplementation((command: string, payload?: { params?: Record<string, unknown> }) => {
      const source = payload?.params?.source;
      if (command === "preview_open") return Promise.resolve({
        source: source ?? "local",
        availability: "ready",
        format: "markdown",
        relativePath: "项目说明.md",
        sizeBytes: source === "baseline" ? 0 : body.length,
        sourceModifiedAt: null,
        contentRevision: source === "baseline" ? emptyRevision : "a".repeat(64),
        contentBytes: source === "baseline" ? 0 : body.length,
        ticketId: source === "baseline" ? "ticket-old" : "ticket-new",
        ticketExpiresAt: 100,
        reason: null,
      });
      if (command === "text_resource_read") return Promise.resolve({ contentRevision: "a".repeat(64), offset: 0, text: body, nextOffset: body.length, endOfResource: true });
      if (command === "text_diff_compute") return Promise.resolve({
        diffTicketId: "diff-1",
        expiresAt: 100,
        left: payload?.params?.left,
        right: payload?.params?.right,
        summary: { addedLines: 1, deletedLines: 0, unchangedLines: 0, changeBlocks: 1 },
        totalRows: 1,
      });
      if (command === "text_diff_read") return Promise.resolve({ cursor: 0, nextCursor: null, rows: [{ kind: "insert", leftLine: null, rightLine: 1, text: "# 新文档", hasLineTerminator: false }] });
      if (command === "file_confirm") return Promise.resolve({ confirmationId: "confirmation-1", confirmedBy: "当前用户", confirmedAt: 1, checksumAlgorithm: "sha256", checksumValue: "a".repeat(64), replayed: false });
      return Promise.resolve();
    });
    const onMessage = vi.fn();
    const onConfirmationChanged = vi.fn();
    const selectedWorkspace = workspace({});
    const selectedFile = file({
      relativePath: "项目说明.md",
      localChangeState: "added",
      confirmationState: "pending",
      checksumAlgorithm: "sha256",
      checksumValue: "a".repeat(64),
      baselineVersionId: null,
      stateRevision: 1,
    });
    render(<WorkspacePage conflicts={[]} connectingWorkspace={false} createWorkspace={() => Promise.resolve()} deliverableBinding={null} detailIntent={0} diffIntent={0} files={[selectedFile]} hostPlatform="macos" loadError={null} loadState="ready" onFile={() => {}} onMessage={onMessage} onOpenAi={() => {}} onConfirmationChanged={onConfirmationChanged} onRetryLoad={() => {}} revision={1} selectedFile={selectedFile} selectedWorkspace={selectedWorkspace} workspaces={[selectedWorkspace]}/>);

    fireEvent.click(screen.getByRole("button", { name: "更多文件操作" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "确认内容" }));
    const dialog = screen.getByRole("dialog", { name: "是否确认文件内容？" });
    expect(within(dialog).queryByRole("button", { name: "确认并提交" })).toBeNull();
    fireEvent.click(within(dialog).getByRole("button", { name: "确认" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("file_confirm", { params: expect.objectContaining({ workspaceId: "ws-1", localFileId: "f1", expectedChecksumValue: "a".repeat(64) }) }));
    await waitFor(() => expect(onConfirmationChanged).toHaveBeenCalled());
    expect(onMessage).toHaveBeenCalledWith("本次文件内容已确认。");
  });

  it("Windows 文件操作菜单使用文件管理器文案和通用定位图标", () => {
    const selectedWorkspace = workspace({});
    const selectedFile = file({});
    const { container } = render(<WorkspacePage conflicts={[]} connectingWorkspace={false} createWorkspace={() => Promise.resolve()} deliverableBinding={null} detailIntent={0} diffIntent={0} files={[selectedFile]} hostPlatform="windows" loadError={null} loadState="ready" onFile={() => {}} onMessage={() => {}} onOpenAi={() => {}} onRetryLoad={() => {}} revision={1} selectedFile={selectedFile} selectedWorkspace={selectedWorkspace} workspaces={[selectedWorkspace]}/>);

    fireEvent.click(screen.getByRole("button", { name: "更多文件操作" }));
    const reveal = screen.getByRole("menuitem", { name: "在文件管理器中显示" });
    expect(reveal.querySelector("svg")?.getAttribute("viewBox")).toBe("0 0 24 24");
    expect(container.textContent).not.toContain("Finder");
  });

  it("云端文件菜单不提供本机打开或文件管理器定位", () => {
    const selectedWorkspace = workspace({ workspaceKind: "cloud_team", localRoot: "cilai-cloud://team-1", teamSpaceId: "team-1" });
    const selectedFile = file({});
    render(<WorkspacePage conflicts={[]} connectingWorkspace={false} createWorkspace={() => Promise.resolve()} deliverableBinding={null} detailIntent={0} diffIntent={0} files={[selectedFile]} hostPlatform="macos" loadError={null} loadState="ready" onFile={() => {}} onMessage={() => {}} onOpenAi={() => {}} onRetryLoad={() => {}} revision={1} selectedFile={selectedFile} selectedWorkspace={selectedWorkspace} workspaces={[selectedWorkspace]}/>);

    expect(screen.getByRole("alert").textContent).toContain("旧云端来源不可用");
    expect(screen.queryByRole("button", { name: "更多文件操作" })).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("本地历史确认和遗留待确认状态都作为普通文件展示", () => {
    const trustedFile = file({
      displayState: "confirmed_ready",
      localChangeState: "added",
      confirmationState: "confirmed",
    });
    const selectedWorkspace = workspace({});
    const rendered = render(<WorkspacePage conflicts={[]} connectingWorkspace={false} createWorkspace={() => Promise.resolve()} deliverableBinding={null} detailIntent={0} diffIntent={0} files={[trustedFile]} hostPlatform="macos" loadError={null} loadState="ready" onFile={() => {}} onMessage={() => {}} onOpenAi={() => {}} onRetryLoad={() => {}} revision={1} selectedFile={trustedFile} selectedWorkspace={selectedWorkspace} workspaces={[selectedWorkspace]}/>);

    expect(rendered.container.querySelector("[data-file-status]")).toBeNull();
    expect(screen.queryByText("本地新增")).toBeNull();

    rendered.rerender(<WorkspacePage conflicts={[]} connectingWorkspace={false} createWorkspace={() => Promise.resolve()} deliverableBinding={null} detailIntent={1} diffIntent={0} files={[trustedFile]} hostPlatform="macos" loadError={null} loadState="ready" onFile={() => {}} onMessage={() => {}} onOpenAi={() => {}} onRetryLoad={() => {}} revision={1} selectedFile={trustedFile} selectedWorkspace={selectedWorkspace} workspaces={[selectedWorkspace]}/>);
    expect(screen.getByRole("complementary", { name: "文件详情" })).not.toBeNull();
    expect(screen.queryByText("文档状态")).toBeNull();
    expect(screen.queryByRole("button", { name: "待确认" })).toBeNull();
    expect(screen.queryByText("本地变化")).toBeNull();

    const legacyFile = file({ displayState: "confirmation_required", localChangeState: "added", confirmationState: "pending" });
    rendered.rerender(<WorkspacePage conflicts={[]} connectingWorkspace={false} createWorkspace={() => Promise.resolve()} deliverableBinding={null} detailIntent={1} diffIntent={0} files={[legacyFile]} hostPlatform="macos" loadError={null} loadState="ready" onFile={() => {}} onMessage={() => {}} onOpenAi={() => {}} onRetryLoad={() => {}} revision={2} selectedFile={legacyFile} selectedWorkspace={selectedWorkspace} workspaces={[selectedWorkspace]}/>);
    expect(screen.queryByRole("button", { name: "待确认" })).toBeNull();
    expect(rendered.container.querySelector("[data-file-status]")).toBeNull();
    expect(screen.queryByText("等待确认")).toBeNull();
  });
});

describe("工作区切换器", () => {
  it("列出工作区，选中项回传 id", () => {
    const onSelect = vi.fn();
    render(<WorkspaceSwitcher
      workspaces={[workspace({}), workspace({ workspaceId: "ws-2", displayName: "另一个工作区" })]}
      selectedWorkspaceId="ws-1"
      projectIndex={projectIndex}
      addWorkspaceDisabled={false}
      onAddWorkspace={() => {}}
      onSelect={onSelect}/>);

    fireEvent.click(screen.getByText("另一个工作区"));
    expect(onSelect).toHaveBeenCalledWith("ws-2");
  });

  it("搜索框按名称过滤", () => {
    render(<WorkspaceSwitcher
      workspaces={[workspace({}), workspace({ workspaceId: "ws-2", displayName: "另一个工作区" })]}
      selectedWorkspaceId="ws-1"
      projectIndex={projectIndex}
      addWorkspaceDisabled={false}
      onAddWorkspace={() => {}}
      onSelect={() => {}}/>);

    fireEvent.click(screen.getByRole("button", { name: "搜索项目或工作区" }));
    fireEvent.change(screen.getByLabelText("搜索项目或工作区"), { target: { value: "另一个项目" } });
    expect(screen.queryByText("示例工作区")).toBeNull();
    expect(screen.getByText("另一个工作区")).toBeDefined();

    expect(screen.queryByRole("button", { name: "关闭工作区搜索" })).toBeNull();
  });

  it("以紧凑单行展示工作区及所属项目且不暴露本地路径", () => {
    render(<WorkspaceSwitcher
      workspaces={[workspace({ localRoot: "/Users/example/private/project" })]}
      selectedWorkspaceId="ws-1"
      projectIndex={new Map([["ws-1", [{ projectId: "project-1", displayName: "统一采集项目" }]]])}
      addWorkspaceDisabled={false}
      onAddWorkspace={() => {}}
      onSelect={() => {}}/>);

    expect(screen.getByText("统一采集项目")).toBeDefined();
    expect(screen.getByText("示例工作区")).toBeDefined();
    expect(screen.queryByText("可用")).toBeNull();
    expect(screen.queryByText("工作区跟随项目切换")).toBeNull();
    expect(screen.queryByText("/Users/example/private/project")).toBeNull();
  });

  it("不展示没有项目归属的工作区", () => {
    render(<WorkspaceSwitcher
      workspaces={[workspace({}), workspace({ workspaceId: "ws-2", displayName: "没有归属的工作区" })]}
      selectedWorkspaceId="ws-1"
      projectIndex={new Map([["ws-1", [{ projectId: "project-1", displayName: "示例项目" }]]])}
      addWorkspaceDisabled={false}
      onAddWorkspace={() => {}}
      onSelect={() => {}}/>);

    expect(screen.getByText("示例工作区")).toBeDefined();
    expect(screen.queryByText("没有归属的工作区")).toBeNull();
    expect(screen.queryByText(/未归属项目/)).toBeNull();
  });

  it("底部通过统一入口添加本地或云端工作空间", () => {
    const onAddWorkspace = vi.fn();
    render(<WorkspaceSwitcher
      addWorkspaceDisabled={false}
      workspaces={[workspace({})]}
      selectedWorkspaceId="ws-1"
      projectIndex={projectIndex}
      onAddWorkspace={onAddWorkspace}
      onSelect={() => {}}/>);

    fireEvent.click(screen.getByRole("button", { name: "添加工作空间" }));
    expect(onAddWorkspace).toHaveBeenCalledTimes(1);
  });
});


it("共享工作空间只显示一行并可通过任一关联项目搜索", () => {
  render(<WorkspaceSwitcher workspaces={[workspace({})]} selectedWorkspaceId="ws-1" projectIndex={new Map([["ws-1", [{ projectId: "a", displayName: "项目甲" }, { projectId: "b", displayName: "项目乙" }]]])} addWorkspaceDisabled={false} onAddWorkspace={vi.fn()} onSelect={vi.fn()}/>);
  expect(screen.getAllByRole("button", { name: "示例工作区，项目甲、项目乙" })).toHaveLength(1);
  fireEvent.click(screen.getByRole("button", { name: "搜索项目或工作区" }));
  fireEvent.change(screen.getByLabelText("搜索项目或工作区"), { target: { value: "项目乙" } });
  expect(screen.getByRole("button", { name: "示例工作区，项目甲、项目乙" })).toBeTruthy();
});
