import { act,cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApplicationsPage } from "./applications";
import type { CapabilityCreationProject } from "./generated/protocol";

const invoke = vi.hoisted(() => vi.fn());
const listen = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke,listen }));
vi.mock("./personal-web-application", () => ({ PersonalWebApplication: ({ title, active, projectId }: { title: string; active: boolean; projectId:string }) => <div hidden={!active} data-personal-project={projectId} data-testid={`installed-${title}`}>{title}内容<iframe title="installed-fixture-frame" src="app://personal-capability/11111111-1111-4111-8111-111111111111"/></div> }));
vi.mock("./personal-capability-editor", () => ({ PersonalCapabilityEditor: ({ onClose }: { onClose: () => void }) => <div role="dialog" aria-label="编辑"><button onClick={onClose}>关闭</button></div> }));

const packaged: CapabilityCreationProject = {
  creationProjectId: "capability_0123456789abcdef0123456789abcdef", title: "文件检查", businessGoal: "检查本地材料",
  targetType: "web_application", status: "packaged", conversationId: null,
  managedRelativePath: "capability-projects/local", requestedPermissions: [], acceptanceExamples: [],
  sourceDigest: `sha256:${"a".repeat(64)}`, sourceFileCount: 1, packageDigest: `sha256:${"b".repeat(64)}`,
  stateRevision: 1, errorReason: null, createdAt: 1, updatedAt: 1,
};
let projects: CapabilityCreationProject[];
beforeEach(() => {
  projects = [packaged]; invoke.mockReset();
  listen.mockReset();listen.mockResolvedValue(()=>{});
  invoke.mockImplementation((command: string, args?: { method?: string }) => {
    if (command === "app_server_request" && args?.method === "capabilityCreation/list") return Promise.resolve({ projects });
    if (command === "personal_capability_upgrade") return Promise.resolve({ project: { ...projects[0], status: "installed", stateRevision: 2 } });
    if (command === "app_server_request" && args?.method === "capabilityCreation/preview") return Promise.resolve({ creationProjectId: packaged.creationProjectId, title: packaged.title, html: "<h1>检查</h1><script>window.parent.postMessage('blocked','*')</script>", contentDigest: "preview" });
    if (command === "personal_application_open") return Promise.resolve({ creationProjectId: packaged.creationProjectId, title: packaged.title, html: "<h1>已安装</h1>", contentDigest: "installed", installationRevision: 1 });
    if (command === "personal_capability_export") return Promise.resolve(true);
    if (command === "personal_applications_close_ack") return Promise.resolve(null);
    return Promise.reject(new Error(`unexpected invocation: ${command}`));
  });
});
afterEach(() => { cleanup(); vi.useRealTimers(); });

async function menu() {
  fireEvent.click(await screen.findByRole("button", { name: "文件检查的更多操作" }));
}

describe("个人应用", () => {
  it("通知只选择仍存在的同一文档，不重新打开或重载应用",async()=>{
    projects=[{...packaged,status:"installed"}];
    const onMessage=vi.fn(),consumed=vi.fn();
    const view=render(<ApplicationsPage onMessage={onMessage} onOpenIntentConsumed={consumed}/>);
    fireEvent.click(await screen.findByRole("button",{name:"文件检查，打开"}));
    const frame=await screen.findByTitle("installed-fixture-frame");
    fireEvent.click(screen.getByRole("button",{name:"应用"}));
    const intent={token:"notification-one",kind:"application" as const,applicationId:packaged.creationProjectId,objectType:"personal-page-notification",objectId:frame.getAttribute("src")!,target:"existing-document"};
    view.rerender(<ApplicationsPage onMessage={onMessage} onOpenIntentConsumed={consumed} openIntent={intent}/>);
    await waitFor(()=>expect(screen.getByRole("button",{name:"文件检查"}).getAttribute("aria-current")).toBe("page"));
    expect(screen.getByTitle("installed-fixture-frame")).toBe(frame);
    expect(invoke.mock.calls.filter(call=>call[0]==="personal_application_open")).toHaveLength(1);
    expect(consumed).toHaveBeenCalledWith("notification-one");
    view.rerender(<ApplicationsPage onMessage={onMessage} onOpenIntentConsumed={consumed} openIntent={{...intent,token:"notification-two",objectId:"app://personal-capability/22222222-2222-4222-8222-222222222222"}}/>);
    await waitFor(()=>expect(onMessage).toHaveBeenCalledWith("通知对应的页面已关闭。","error"));
    expect(invoke.mock.calls.filter(call=>call[0]==="personal_application_open")).toHaveLength(1);
    expect(screen.getByTitle("installed-fixture-frame")).toBe(frame);
    fireEvent.click(screen.getByRole("button",{name:"关闭文件检查"}));
    view.rerender(<ApplicationsPage onMessage={onMessage} onOpenIntentConsumed={consumed} openIntent={{...intent,token:"notification-three"}}/>);
    await waitFor(()=>expect(consumed).toHaveBeenCalledWith("notification-three"));
    expect(screen.queryByTitle("installed-fixture-frame")).toBeNull();
    expect(invoke.mock.calls.filter(call=>call[0]==="personal_application_open")).toHaveLength(1);
  });
  it("维护关闭事件先卸载个人页面，再确认关闭",async()=>{
    let receive!:(value:{token:string})=>void;
    listen.mockImplementation(async(_event,handler)=>{receive=handler;return()=>{};});
    projects=[{...packaged,status:"installed"}];render(<ApplicationsPage onMessage={vi.fn()}/>);
    fireEvent.click(await screen.findByRole("button",{name:"文件检查，打开"}));await screen.findByTestId("installed-文件检查");
    const token="11111111-1111-4111-8111-111111111111";
    act(()=>receive({token}));
    await waitFor(()=>expect(invoke).toHaveBeenCalledWith("personal_applications_close_ack",{token}));
    expect(screen.queryByTestId("installed-文件检查")).toBeNull();
  });
  it("直接读取本地应用，提供创建且不调用企业目录", async () => {
    const create = vi.fn(); render(<ApplicationsPage onMessage={vi.fn()} onCreate={create}/>);
    await screen.findByRole("button", { name: "文件检查，安装" });
    expect(invoke.mock.calls).toEqual([["app_server_request", { method: "capabilityCreation/list", params: {} }]]);
    expect(screen.queryByText("企业应用")).toBeNull();
    expect(screen.queryByText("我的创建")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "创建" })); expect(create).toHaveBeenCalledOnce();
    await menu(); expect(screen.getAllByRole("menuitem").map(item => item.textContent)).toEqual(["编辑", "预览", "导出"]);
  });

  it("读取失败明确显示错误并支持重试，不冒充空目录", async () => {
    invoke.mockRejectedValueOnce(new Error("本地资料读取失败"));
    render(<ApplicationsPage onMessage={vi.fn()}/>);
    await screen.findByText("无法读取应用"); expect(screen.queryByText("还没有应用")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "重试" }));
    await screen.findByRole("button", { name: "文件检查，安装" });
  });

  it("搜索只筛选已读取的本地列表", async () => {
    projects = [packaged, { ...packaged, creationProjectId: "capability_other", title: "知识卡片", status: "planned", packageDigest: null }];
    render(<ApplicationsPage onMessage={vi.fn()}/>); await screen.findByText("知识卡片");
    fireEvent.change(screen.getByRole("searchbox", { name: "搜索应用" }), { target: { value: "知识" } });
    expect(screen.queryByText("文件检查")).toBeNull(); expect(invoke).toHaveBeenCalledOnce();
  });

  it("未安装页面只按只读沙箱预览，不走企业应用容器", async () => {
    render(<ApplicationsPage onMessage={vi.fn()}/>); await menu();
    fireEvent.click(screen.getByRole("menuitem", { name: "预览" }));
    const frame = await screen.findByTitle("文件检查");
    expect(frame.getAttribute("sandbox")).toBe(""); expect(frame.getAttribute("srcdoc")).toContain("script-src 'none'");
    expect(invoke.mock.calls.every(([command]) => !String(command).startsWith("application_"))).toBe(true);
  });

  it("安装成功后通过个人应用入口打开，再次打开保留原页面", async () => {
    const onMessage = vi.fn(); render(<ApplicationsPage onMessage={onMessage}/>);
    fireEvent.click(await screen.findByRole("button", { name: "文件检查，安装" }));
    expect(invoke.mock.calls.some(([command]) => command === "personal_capability_upgrade")).toBe(false);
    fireEvent.click(await screen.findByRole("button", { name: "安装" }));
    await waitFor(() => expect(onMessage).toHaveBeenCalledWith("「文件检查」已安装。"));
    projects = [{ ...packaged, status: "installed" }];
    fireEvent.click(screen.getByRole("button", { name: "文件检查，打开" }));
    const content = await screen.findByTestId("installed-文件检查");
    fireEvent.click(screen.getByRole("button", { name: "应用" }));
    fireEvent.click(await screen.findByRole("button", { name: "文件检查，打开" }));
    expect(screen.getByTestId("installed-文件检查")).toBe(content);
    expect(invoke.mock.calls.filter(([command]) => command === "personal_application_open")).toHaveLength(1);
  });

  it("安装前显示精确权限，取消不安装，确认携带版本与权限", async () => {
    projects = [{ ...packaged, targetType: "web_mcp_application", requestedPermissions: ["personal_file.read", "workbench_artifact.create"] }];
    render(<ApplicationsPage onMessage={vi.fn()}/>);
    fireEvent.click(await screen.findByRole("button", { name: "文件检查，安装" }));
    expect(screen.getByText("读取选定文件")).toBeTruthy();
    expect(screen.getByText("创建成果")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(invoke.mock.calls.some(([command]) => command === "personal_capability_upgrade")).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "文件检查，安装" }));
    fireEvent.click(screen.getByRole("button", { name: "安装" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("personal_capability_upgrade", { request: {
      creationProjectId: packaged.creationProjectId, expectedRevision: 1, packageDigest: packaged.packageDigest,
      confirmedPermissions: ["personal_file.read", "workbench_artifact.create"], clientOperationId: expect.any(String),
    } }));
    expect(invoke.mock.calls.some(([, args]) => args?.method === "capabilityCreation/install")).toBe(false);
  });

  it("安装回执失败保留应用并提示未确认，不自动重做安装", async () => {
    const message = vi.fn();
    render(<ApplicationsPage onMessage={message}/>);
    fireEvent.click(await screen.findByRole("button", { name: "文件检查，安装" }));
    invoke.mockRejectedValueOnce(new Error("保存结果未知"));
    fireEvent.click(screen.getByRole("button", { name: "安装" }));
    await waitFor(() => expect(message).toHaveBeenCalledWith(expect.stringContaining("安装未确认"), "error"));
    expect(screen.getByRole("button", { name: "文件检查，安装" })).toBeTruthy();
    expect(invoke.mock.calls.filter(([command]) => command === "personal_capability_upgrade")).toHaveLength(1);
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("导出调用个人安装包入口，成功后显示真实保存结果", async () => {
    const message = vi.fn(); render(<ApplicationsPage onMessage={message}/>); await menu();
    fireEvent.click(screen.getByRole("menuitem", { name: "导出" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("personal_capability_export", { creationProjectId: packaged.creationProjectId }));
    await waitFor(() => expect(message).toHaveBeenCalledWith("安装包已导出。", "info"));
    expect(invoke.mock.calls.some(([command]) => String(command).includes("review") || String(command).includes("submit"))).toBe(false);
  });

  it("取消导出不显示成功，失败保留应用并显示原因", async () => {
    const message = vi.fn(); render(<ApplicationsPage onMessage={message}/>); await menu();
    invoke.mockResolvedValueOnce(false); fireEvent.click(screen.getByRole("menuitem", { name: "导出" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "文件检查的更多操作" }).hasAttribute("disabled")).toBe(false));
    expect(message).not.toHaveBeenCalled(); await menu();
    invoke.mockRejectedValueOnce(new Error("保存失败")); fireEvent.click(screen.getByRole("menuitem", { name: "导出" }));
    await waitFor(() => expect(message).toHaveBeenCalledWith(expect.stringContaining("保存失败"), "error"));
    expect(screen.getByText("文件检查")).toBeTruthy();
  });

  it("隐藏时不读取，激活后读取一次，焦点和时间不会刷新企业目录", async () => {
    const view = render(<ApplicationsPage active={false} onMessage={vi.fn()}/>); expect(invoke).not.toHaveBeenCalled();
    view.rerender(<ApplicationsPage active onMessage={vi.fn()}/>); await screen.findByText("文件检查");
    vi.useFakeTimers(); fireEvent.focus(window); fireEvent(document, new Event("visibilitychange"));
    vi.advanceTimersByTime(120_000); expect(invoke).toHaveBeenCalledOnce();
  });

  it("本地资料变化后拒绝迟到目录覆盖", async () => {
    const responses: Array<(value: { projects: CapabilityCreationProject[] }) => void> = [];
    invoke.mockImplementation(() => new Promise(resolve => responses.push(resolve)));
    const view = render(<ApplicationsPage profileId="first" onMessage={vi.fn()}/>);
    await waitFor(() => expect(responses).toHaveLength(1));
    view.rerender(<ApplicationsPage profileId="second" onMessage={vi.fn()}/>);
    await waitFor(() => expect(responses).toHaveLength(2));
    responses[1]({ projects: [{ ...packaged, title: "当前应用" }] }); await screen.findByText("当前应用");
    responses[0]({ projects: [packaged] });
    await waitFor(() => expect(screen.queryByText("文件检查")).toBeNull());
  });

  it("企业来源搜索结果不触发远端启动", async () => {
    const message = vi.fn(), consumed = vi.fn();
    render(<ApplicationsPage onMessage={message} onOpenIntentConsumed={consumed} openIntent={{ token: "old", kind: "application", applicationId: "enterprise-old", objectType: "record", objectId: "old", target: "detail" }}/>);
    await waitFor(() => expect(consumed).toHaveBeenCalledWith("old"));
    expect(message).toHaveBeenCalledWith("应用不可用。", "error");
    expect(invoke).toHaveBeenCalledOnce();
  });
});
