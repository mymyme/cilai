import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { type AiMemorySettings } from "./ai-runtime";
import { MemorySettingsPage, SettingsPage } from "./settings";
import { invoke } from "./host";
vi.mock("./host", () => ({ invoke: vi.fn(), listen: vi.fn(async () => () => {}) }));
vi.mock("./personal-memory-reset",()=>({PersonalMemoryReset:({disabled}:{disabled:boolean})=><button disabled={disabled}>删除</button>}));
const call = vi.mocked(invoke);
const original: AiMemorySettings = { lifecycleReady: true, enabled: true, fromToolChats: true, running: false, hasMemory: true, modelLabel: "企业模型乙", lastError: null, phase: "available", lastUpdatedAt: 123 };
beforeEach(() => { call.mockReset(); call.mockResolvedValue({ ...original }); });
afterEach(cleanup);

describe("记忆设置", () => {
  it("原生文件存在只显示已有记忆和文件时间，不宣称整理成功", async () => {
    render(<MemorySettingsPage onOpenModels={vi.fn()}/>);
    await screen.findByText("已有记忆");
    expect(screen.getByText(/记忆文件更新/)).toBeTruthy();
    expect(screen.queryByText("记忆已更新")).toBeNull();
    expect(screen.queryByText("正在整理记忆")).toBeNull();
    expect(screen.queryByText(/最近更新/)).toBeNull();
    expect((screen.getByRole("switch", { name: "启用记忆" }) as HTMLButtonElement).disabled).toBe(false);
  });
  it("没有原生文件时显示尚无记忆，不推断后台阶段", async () => {
    call.mockResolvedValue({ ...original, phase: "empty", hasMemory: false, lastUpdatedAt: null });
    render(<MemorySettingsPage onOpenModels={vi.fn()}/>);
    await screen.findByText("尚无记忆");
    expect(screen.queryByText(/记忆文件更新/)).toBeNull();
    expect(screen.queryByText("等待提炼")).toBeNull();
    expect(screen.queryByText("正在提炼记忆")).toBeNull();
    expect(screen.queryByText("记忆已更新")).toBeNull();
  });
  it("关闭时保留已有文件时间和删除入口，不显示后台完成状态", async () => {
    call.mockResolvedValue({ ...original, enabled: false, phase: "disabled" });
    render(<MemorySettingsPage onOpenModels={vi.fn()}/>);
    await screen.findByText("记忆已关闭");
    expect(screen.getByText(/记忆文件更新/)).toBeTruthy();
    expect(screen.getByRole("switch", { name: "启用记忆" }).getAttribute("aria-checked")).toBe("false");
    expect((screen.getByRole("switch", { name: "从工具会话中提炼" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "删除" }) as HTMLButtonElement).disabled).toBe(false);
  });
  it("读取实际状态，保存关闭设置后停止启用工具会话开关", async () => {
    call.mockImplementation(async (command, args) => {
      if (command !== "ai_memory_settings_save") return { ...original };
      const settings = (args as { settings: Pick<AiMemorySettings, "enabled" | "fromToolChats"> }).settings;
      return { ...original, ...settings, phase: settings.enabled ? "available" : "disabled" };
    });
    render(<MemorySettingsPage onOpenModels={vi.fn()}/>);
    await screen.findByText("企业模型乙");
    fireEvent.click(screen.getByRole("switch", { name: "启用记忆" }));
    await screen.findByText("已保存，下次运行生效");
    expect(call).toHaveBeenCalledWith("ai_memory_settings_save", { settings: { enabled: false, fromToolChats: true } });
    expect(screen.getByText("记忆已关闭")).toBeTruthy();
    expect(screen.getByRole("switch", { name: "启用记忆" }).getAttribute("aria-checked")).toBe("false");
    expect((screen.getByRole("switch", { name: "从工具会话中提炼" }) as HTMLButtonElement).disabled).toBe(true);
  });
  it("保存失败保留原开关，不显示成功", async () => {
    call.mockImplementation(async (command) => { if (command === "ai_memory_settings_save") throw new Error("保存失败"); return { ...original }; });
    render(<MemorySettingsPage onOpenModels={vi.fn()}/>);
    await screen.findByText("企业模型乙");
    fireEvent.click(screen.getByRole("switch", { name: "启用记忆" }));
    await screen.findByRole("alert");
    expect(screen.getByRole("switch", { name: "启用记忆" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.queryByText("已保存，下次运行生效")).toBeNull();
  });
  it("读取失败时禁用操作，并允许重新读取", async () => {
    call.mockRejectedValueOnce(new Error("读取失败"));
    render(<MemorySettingsPage onOpenModels={vi.fn()}/>);
    await screen.findByRole("alert");
    expect((screen.getByRole("switch", { name: "启用记忆" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "重试" }));
    await screen.findByText("企业模型乙");
  });
  it("前台运行时保留删除预览入口，模型入口直达共用设置", async () => {
    call.mockResolvedValue({ ...original, running: true });
    const open = vi.fn();
    render(<MemorySettingsPage onOpenModels={open}/>);
    await screen.findByText("已有记忆");
    expect((screen.getByRole("button", { name: "删除" }) as HTMLButtonElement).disabled).toBe(false);
    expect(screen.queryByText("正在整理记忆")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /记忆模型/ }));
    expect(open).toHaveBeenCalledOnce();
  });
  it("原生代理错误保留已有文件和时间，不宣称整批更新失败", async () => {
    call.mockResolvedValue({ ...original, lastError: "后台模型请求被拒绝" });
    render(<MemorySettingsPage onOpenModels={vi.fn()}/>);
    await screen.findByText("后台模型请求被拒绝");
    expect(screen.getByText("已有记忆")).toBeTruthy();
    expect(screen.getByText(/记忆文件更新/)).toBeTruthy();
    expect(screen.queryByText("记忆更新失败")).toBeNull();
    expect(screen.queryByText("部分记忆提炼失败")).toBeNull();
  });
  it("尚无文件时原样展示请求错误，不推断首次整理结果", async () => {
    call.mockResolvedValue({ ...original, hasMemory: false, lastUpdatedAt: null, phase: "empty", lastError: "模型工具协议不兼容" });
    render(<MemorySettingsPage onOpenModels={vi.fn()}/>);
    await screen.findByText("模型工具协议不兼容");
    expect(screen.getByText("尚无记忆")).toBeTruthy();
    expect(screen.queryByText("记忆更新失败")).toBeNull();
    expect(screen.queryByText(/记忆文件更新/)).toBeNull();
  });
  it("结构化代理错误保留简洁原因和可展开安全诊断，不触发新请求", async () => {
    const lastError = "原生记忆请求失败：" + JSON.stringify({
      error: { code: "AI-TOOL-001", title: "模型工具协议不兼容", message: "上游拒绝工具选择参数。", diagnostics: { parameter: "tool_choice", upstream_status: 400, upstream_request_id: "upstream-123", unexpected: "不可展示字段" } },
      request_id: "request-123",
    });
    call.mockResolvedValue({ ...original, lastError });
    render(<MemorySettingsPage onOpenModels={vi.fn()}/>);
    await screen.findByText("原生记忆请求失败：模型工具协议不兼容");
    expect(screen.getByText("已有记忆")).toBeTruthy();
    const details = screen.getByText("详情").closest("details")!;
    expect(details.open).toBe(false);
    fireEvent.click(screen.getByText("详情"));
    expect(details.open).toBe(true);
    expect(within(details).getByText("tool_choice")).toBeTruthy();
    expect(within(details).getByText("request-123")).toBeTruthy();
    expect(within(details).getByText("upstream-123")).toBeTruthy();
    expect(screen.queryByText("不可展示字段")).toBeNull();
    expect(screen.queryByText(lastError)).toBeNull();
    expect(screen.getByText(/记忆文件更新/)).toBeTruthy();
    fireEvent.click(screen.getByText("详情"));
    expect(details.open).toBe(false);
    expect(call).toHaveBeenCalledTimes(1);
  });
  it.each(["上游错误：{响应不完整", '{"error":{"message":42}}'])
    ("无法识别的错误保留原始信息：%s", async (lastError) => {
      call.mockResolvedValue({ ...original, lastError });
      render(<MemorySettingsPage onOpenModels={vi.fn()}/>);
      await screen.findByText(lastError);
      expect(screen.queryByText("详情")).toBeNull();
    });
  it("二级导航包含模型与归档入口", async () => {
    const change = vi.fn();
    render(<SettingsPage section="memory" onSectionChange={change} workspace={null} hostPlatform="macos" projects={[]} onMessage={vi.fn()}/>);
    const nav = screen.getByRole("navigation", { name: "设置导航" });
    fireEvent.click(within(nav).getByRole("button", { name: "已归档会话" }));
    expect(change).toHaveBeenLastCalledWith("archived");
    fireEvent.click(within(nav).getByRole("button", { name: "模型设置" }));
    expect(change).toHaveBeenLastCalledWith("agents");
    fireEvent.click(within(nav).getByRole("button", { name: "软件更新" }));
    expect(change).toHaveBeenLastCalledWith("updates");
    await waitFor(() => expect(screen.getByText("企业模型乙")).toBeTruthy());
  });
});


describe("个人记忆启用确认",()=>{
  const service={bindingId:"memory",bindingRevision:2,connectionId:"local-connection",connectionName:"个人连接",configRevision:3,authGeneration:1,modelId:"selected-memory-model",serviceName:"实际记忆服务",baseUrl:"https://memory.example.invalid/v1",historyScope:"合格的本地历史对话",toolChatsHistoryScope:"历史对话和其中的工具上下文",mayIncurCost:true,consentDigest:"digest-current"};
  it("开启前披露实际服务和历史范围，取消不保存",async()=>{
    call.mockResolvedValue({...original,enabled:false,fromToolChats:false,phase:"disabled",service});
    render(<MemorySettingsPage onOpenModels={vi.fn()}/>);await screen.findByText("记忆已关闭");
    fireEvent.click(screen.getByRole("switch",{name:"启用记忆"}));
    const dialog=screen.getByRole("dialog");expect(dialog.textContent).toContain("实际记忆服务");expect(dialog.textContent).toContain("合格的本地历史对话");expect(dialog.textContent).toContain("可能产生服务费用");
    fireEvent.click(within(dialog).getByRole("button",{name:"取消"}));
    expect(call.mock.calls.some(([command])=>command==="ai_memory_settings_save")).toBe(false);
  });
  it("启用提交实际披露摘要与历史选项，回执未知不自动重发",async()=>{
    call.mockImplementation(async(command)=>{if(command==="ai_memory_settings_save")throw new Error("model_memory_confirmation_changed");return {...original,enabled:false,fromToolChats:false,phase:"disabled",service};});
    render(<MemorySettingsPage onOpenModels={vi.fn()}/>);await screen.findByText("记忆已关闭");
    fireEvent.click(screen.getByRole("switch",{name:"启用记忆"}));fireEvent.click(within(screen.getByRole("dialog")).getByRole("button",{name:"启用"}));
    await screen.findByRole("alert");const args=call.mock.calls.find(([command])=>command==="ai_memory_settings_save")?.[1] as {settings:{confirmation:{consentDigest:string;fromToolChats:boolean;confirmedAt:number}}};
    expect(args.settings.confirmation.consentDigest).toBe("digest-current");expect(args.settings.confirmation.fromToolChats).toBe(false);expect(args.settings.confirmation.confirmedAt).toBeGreaterThan(0);
    expect((screen.getByRole("switch",{name:"启用记忆"}) as HTMLButtonElement).disabled).toBe(true);
    expect(call.mock.calls.filter(([command])=>command==="ai_memory_settings_save")).toHaveLength(1);
    fireEvent.click(screen.getByRole("button",{name:"刷新状态"}));await waitFor(()=>expect((screen.getByRole("switch",{name:"启用记忆"}) as HTMLButtonElement).disabled).toBe(false));
  });
  it("扩大工具历史范围需再次确认，缩小范围可直接保存",async()=>{
    call.mockImplementation(async(command,args)=>command==="ai_memory_settings_save"?{...original,...(args as {settings:object}).settings,service}:{...original,fromToolChats:false,service});
    render(<MemorySettingsPage onOpenModels={vi.fn()}/>);await screen.findByText("已有记忆");fireEvent.click(screen.getByRole("switch",{name:"从工具会话中提炼"}));
    expect(screen.getByRole("dialog").textContent).toContain("历史对话和其中的工具上下文");fireEvent.click(within(screen.getByRole("dialog")).getByRole("button",{name:"启用"}));await screen.findByText("已保存，下次运行生效");
    fireEvent.click(screen.getByRole("switch",{name:"从工具会话中提炼"}));await waitFor(()=>expect(call.mock.calls.filter(([command])=>command==="ai_memory_settings_save")).toHaveLength(2));expect(screen.queryByRole("dialog")).toBeNull();
  });
  it("没有实际记忆用途时不能启用，不沿用旧审核模型",async()=>{
    call.mockResolvedValue({...original,enabled:false,fromToolChats:false,phase:"disabled",service:null,modelLabel:null});render(<MemorySettingsPage onOpenModels={vi.fn()}/>);await screen.findByText("记忆已关闭");
    fireEvent.click(screen.getByRole("switch",{name:"启用记忆"}));await screen.findByRole("alert");expect(call.mock.calls.some(([command])=>command==="ai_memory_settings_save")).toBe(false);expect(screen.queryByRole("dialog")).toBeNull();
  });
});
