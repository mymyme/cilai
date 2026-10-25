import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const rpc = vi.hoisted(() => vi.fn());
const sha256 = vi.hoisted(() => vi.fn().mockResolvedValue("a".repeat(64)));
const invoke = vi.hoisted(() => vi.fn());
vi.mock("./app-server-rpc", () => ({ rpc, sha256 }));
vi.mock("./host", () => ({ invoke }));

const { SkillsPage } = await import("./skills");

const userSkill = {
  skillId: "skill_0123456789abcdef0123456789abcdef",
  name: "周报整理",
  description: "整理项目周报",
  source: "user",
  enabled: true,
  automatic: false,
  stateRevision: 3,
  updatedAt: 10,
};

const systemSkill = {
  ...userSkill,
  skillId: "system.skill-creator",
  name: "技能创建器",
  description: "创建可复用技能",
  source: "system",
  automatic: true,
};

describe("技能管理页", () => {
  beforeEach(() => {
    rpc.mockReset();
    invoke.mockReset();
    rpc.mockImplementation(async (method: string, params: Record<string, unknown>) => {
      if (method === "skill/list") {
        return {
          skills: [
            userSkill,
            systemSkill,
          ],
        };
      }
      if (method === "skill/get") {
        return { skill: {
          ...(params.skillId === systemSkill.skillId ? systemSkill : userSkill),
          instructions: "先读取参考结构，再输出进展、风险与计划。",
          files: [
            { path: "references/周报结构.md", category: "references", content: "# 周报结构\n\n完成事项、风险、下周计划。", sizeBytes: 57 },
            { path: "scripts/check.py", category: "scripts", content: "print('check')\n", sizeBytes: 15 },
          ],
          createdAt: 1,
        } };
      }
      throw new Error(`unexpected method: ${method}`);
    });
  });

  afterEach(() => cleanup());

  test("默认展示我的技能，可查看详情", async () => {
    render(<SkillsPage onMessage={vi.fn()} onOpenConversation={vi.fn()}/>);

    expect(await screen.findByText("周报整理")).toBeTruthy();
    expect(screen.queryByText("个人能力")).toBeNull();
    expect(screen.queryByText("技能创建器")).toBeNull();
    expect(screen.getByRole("tab", { name: "我的技能" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("tabpanel", { name: "我的技能" })).toBeTruthy();
    fireEvent.click(screen.getByText("周报整理"));
    expect(await screen.findByText(/先读取参考结构，再输出进展、风险与计划/)).toBeTruthy();
    expect(screen.getByText("已启用")).toBeTruthy();
    expect(screen.getByText("未开启")).toBeTruthy();
    expect(screen.getByRole("button", { name: "停用" })).toBeTruthy();
    expect(screen.getByRole("dialog", { name: "周报整理" }).querySelectorAll(".skill-toggle")).toHaveLength(1);
    expect(screen.queryByText("主文件")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "文件" }));
    const referencesFolder = screen.getByRole("button", { name: "references" });
    const referenceFile = screen.getByRole("button", { name: /周报结构.md/ });
    expect(referenceFile.classList.contains("skill-file-item")).toBe(true);
    fireEvent.click(referencesFolder);
    expect(screen.queryByRole("button", { name: /周报结构.md/ })).toBeNull();
    fireEvent.click(referencesFolder);
    fireEvent.click(screen.getByRole("button", { name: /周报结构.md/ }));
    expect(await screen.findByText(/完成事项、风险、下周计划/)).toBeTruthy();
    expect(rpc).toHaveBeenCalledWith("skill/get", { skillId: userSkill.skillId });
  });

  test("系统技能只显示列表，点击或按键不打开详情或执行操作", async () => {
    const onOpenConversation = vi.fn();
    render(<SkillsPage onMessage={vi.fn()} onOpenConversation={onOpenConversation}/>);
    await screen.findByText("周报整理");
    const systemTab = screen.getByRole("tab", { name: "系统技能" });
    expect(within(systemTab).getByText("1")).toBeTruthy();
    fireEvent.click(systemTab);
    expect(systemTab.getAttribute("aria-selected")).toBe("true");
    const panel = screen.getByRole("tabpanel", { name: "系统技能" });
    expect(within(panel).queryByText("周报整理")).toBeNull();
    expect(within(panel).getByText("创建可复用技能")).toBeTruthy();
    expect(within(panel).queryByRole("button")).toBeNull();
    expect(panel.querySelector("a, input, [tabindex], [role='button']")).toBeNull();
    expect(screen.queryByRole("button", { name: "添加" })).toBeNull();
    const name = within(panel).getByText("技能创建器");
    fireEvent.click(name);
    fireEvent.keyDown(name, { key: "Enter" });
    fireEvent.keyDown(name, { key: " " });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(onOpenConversation).not.toHaveBeenCalled();
    expect(rpc.mock.calls.every(([method]) => method === "skill/list")).toBe(true);
  });

  test("搜索框可清空关键词，并保留系统分类且不打开详情", async () => {
    render(<SkillsPage onMessage={vi.fn()} onOpenConversation={vi.fn()}/>);
    await screen.findByText("周报整理");
    fireEvent.click(screen.getByRole("tab", { name: "系统技能" }));
    const search = screen.getByRole("searchbox", { name: "搜索技能" });
    fireEvent.change(search, { target: { value: "无匹配" } });
    expect(screen.getByText("没有匹配的技能")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "清空关键词" }));
    expect((search as HTMLInputElement).value).toBe("");
    expect(screen.getByRole("tab", { name: "系统技能" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByText("技能创建器")).toBeTruthy();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(rpc.mock.calls.every(([method]) => method === "skill/list")).toBe(true);
  });

  test("搜索只匹配当前分类，切换与刷新保留搜索和分类数量", async () => {
    render(<SkillsPage onMessage={vi.fn()} onOpenConversation={vi.fn()}/>);
    await screen.findByText("周报整理");
    const search = screen.getByRole("searchbox", { name: "搜索技能" });
    fireEvent.change(search, { target: { value: "可复用" } });
    expect(screen.getByText("没有匹配的技能")).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: "系统技能" }));
    expect(screen.getByText("技能创建器")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "刷新" }));
    await screen.findByText("技能创建器");
    expect((search as HTMLInputElement).value).toBe("可复用");
    expect(screen.getByRole("tab", { name: "系统技能" }).getAttribute("aria-selected")).toBe("true");
    fireEvent.click(screen.getByRole("tab", { name: "我的技能" }));
    expect(screen.getByText("没有匹配的技能")).toBeTruthy();
    expect(within(screen.getByRole("tab", { name: "我的技能" })).getByText("1")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "清除搜索" }));
    expect(screen.getByText("周报整理")).toBeTruthy();
  });

  test("键盘切换分类并关闭添加菜单", async () => {
    render(<SkillsPage onMessage={vi.fn()} onOpenConversation={vi.fn()}/>);
    await screen.findByText("周报整理");
    fireEvent.click(screen.getByRole("button", { name: "添加" }));
    const userTab = screen.getByRole("tab", { name: "我的技能" });
    const systemTab = screen.getByRole("tab", { name: "系统技能" });
    userTab.focus();
    fireEvent.keyDown(userTab, { key: "ArrowRight" });
    expect(document.activeElement).toBe(systemTab);
    expect(userTab.tabIndex).toBe(-1);
    expect(systemTab.tabIndex).toBe(0);
    expect(screen.queryByRole("menu")).toBeNull();
    fireEvent.keyDown(systemTab, { key: "Home" });
    expect(document.activeElement).toBe(userTab);
    expect(screen.queryByRole("menu")).toBeNull();
    fireEvent.keyDown(userTab, { key: "ArrowLeft" });
    expect(document.activeElement).toBe(systemTab);
    fireEvent.keyDown(systemTab, { key: "ArrowRight" });
    expect(document.activeElement).toBe(userTab);
    fireEvent.keyDown(userTab, { key: "End" });
    expect(document.activeElement).toBe(systemTab);
  });

  test("两类技能分别呈现空状态", async () => {
    rpc.mockResolvedValue({ skills: [] });
    render(<SkillsPage onMessage={vi.fn()} onOpenConversation={vi.fn()}/>);
    await screen.findByText("还没有个人技能");
    expect(screen.getByRole("button", { name: "手动创建" })).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: "系统技能" }));
    expect(screen.getByText("暂无系统技能")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "手动创建" })).toBeNull();
  });

  test("用 AI 创建会选中系统技能创建器并进入对话", async () => {
    const onOpenConversation = vi.fn();
    render(<SkillsPage onMessage={vi.fn()} onOpenConversation={onOpenConversation}/>);
    await screen.findByText("周报整理");

    fireEvent.click(screen.getByRole("button", { name: "添加" }));
    fireEvent.click(screen.getByRole("menuitem", { name: /用 AI 创建/ }));

    expect(onOpenConversation).toHaveBeenCalledWith("system.skill-creator", "技能创建器", "create");
  });

  test("删除确认独占交互，取消后保留资源位置并把焦点还给详情", async () => {
    render(<SkillsPage onMessage={vi.fn()} onOpenConversation={vi.fn()}/>);
    fireEvent.click(await screen.findByText("周报整理"));
    const detail = await screen.findByRole("dialog", { name: "周报整理" });
    fireEvent.click(within(detail).getByRole("button", { name: "文件" }));
    fireEvent.click(within(detail).getByRole("button", { name: /周报结构.md/ }));
    fireEvent.click(within(detail).getByLabelText("更多技能操作"));
    const trigger = within(detail).getByRole("button", { name: "删除" });
    trigger.focus();
    fireEvent.click(trigger);

    const confirmation = screen.getByRole("dialog", { name: "删除这个技能？" });
    expect(screen.queryByRole("dialog", { name: "周报整理" })).toBeNull();
    expect(detail.hasAttribute("inert")).toBe(true);
    const cancel = within(confirmation).getByRole("button", { name: "取消" });
    const confirm = within(confirmation).getByRole("button", { name: "删除" });
    for (const button of [cancel, confirm]) {
      Object.defineProperty(button, "offsetParent", { get: () => confirmation, configurable: true });
    }
    confirm.focus();
    fireEvent.keyDown(window, { key: "Tab" });
    expect(document.activeElement).toBe(cancel);
    fireEvent.keyDown(window, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(confirm);

    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "删除这个技能？" })).toBeNull();
    expect(screen.getByRole("dialog", { name: "周报整理" })).toBe(detail);
    expect(detail.hasAttribute("inert")).toBe(false);
    expect(within(detail).getByText(/完成事项、风险、下周计划/)).toBeTruthy();
    expect(document.activeElement).toBe(trigger);

    fireEvent.click(trigger);
    fireEvent.click(within(screen.getByRole("dialog", { name: "删除这个技能？" })).getByRole("button", { name: "取消" }));
    expect(screen.getByRole("dialog", { name: "周报整理" })).toBe(detail);
    expect(rpc.mock.calls.some(([method]) => method === "skill/delete")).toBe(false);
  });

  test("确认删除只提交一次，等待期间不能关闭，成功后更新列表", async () => {
    let finishDelete!: () => void;
    const originalRpc = rpc.getMockImplementation()!;
    rpc.mockImplementation((method: string, params: unknown) => method === "skill/delete"
      ? new Promise<void>((resolve) => { finishDelete = resolve; })
      : originalRpc(method, params));
    const onMessage = vi.fn();
    render(<SkillsPage onMessage={onMessage} onOpenConversation={vi.fn()}/>);
    fireEvent.click(await screen.findByText("周报整理"));
    fireEvent.click(within(await screen.findByRole("dialog", { name: "周报整理" })).getByLabelText("更多技能操作"));
    fireEvent.click(within(screen.getByRole("dialog", { name: "周报整理" })).getByRole("button", { name: "删除" }));
    const confirmation = screen.getByRole("dialog", { name: "删除这个技能？" });
    fireEvent.click(within(confirmation).getByRole("button", { name: "删除" }));
    await waitFor(() => expect(rpc).toHaveBeenCalledWith("skill/delete", expect.objectContaining({
      skillId: userSkill.skillId, expectedRevision: 3,
    })));
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.getByRole("dialog", { name: "删除这个技能？" })).toBe(confirmation);
    expect((within(confirmation).getByRole("button", { name: "处理中…" }) as HTMLButtonElement).disabled).toBe(true);
    expect(rpc.mock.calls.filter(([method]) => method === "skill/delete")).toHaveLength(1);

    finishDelete();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.queryByText("周报整理")).toBeNull();
    expect(onMessage).toHaveBeenCalledWith("技能已删除。");
  });

  test("资源文件只在详情中查看，编辑主文件时由服务端保留", async () => {
    rpc.mockImplementation(async (method: string, params: Record<string, any>) => {
      if (method === "skill/list") return { skills: [userSkill] };
      if (method === "skill/get") return { skill: {
        ...userSkill,
        instructions: "先读取参考结构。",
        files: [{ path: "references/周报结构.md", category: "references", content: "# 结构", sizeBytes: 8 }],
        createdAt: 1,
      } };
      if (method === "skill/save") return { skill: {
        ...userSkill,
        ...params,
        source: "user",
        stateRevision: 4,
        updatedAt: 11,
        createdAt: 1,
        files: [{ path: "references/周报结构.md", category: "references", content: "# 结构", sizeBytes: 8 }],
      } };
      throw new Error(`unexpected method: ${method}`);
    });
    render(<SkillsPage onMessage={vi.fn()} onOpenConversation={vi.fn()}/>);

    fireEvent.click(await screen.findByText("周报整理"));
    fireEvent.click(await screen.findByRole("button", { name: "编辑" }));
    expect(screen.queryByText("资源文件")).toBeNull();
    expect(screen.queryByRole("button", { name: "添加资源" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(rpc).toHaveBeenCalledWith("skill/save", expect.objectContaining({
      files: null,
    })));
  });

  test("可从 ZIP 安装完整技能包", async () => {
    invoke.mockResolvedValue("/tmp/weekly.zip");
    rpc.mockImplementation(async (method: string, params: Record<string, any>) => {
      if (method === "skill/list") return { skills: [userSkill] };
      if (method === "skill/import") return { skill: {
        ...userSkill,
        skillId: "skill_fedcba9876543210fedcba9876543210",
        name: "导入技能",
        description: "从 ZIP 安装",
        instructions: "读取引用后执行。",
        files: [{ path: "references/guide.md", category: "references", content: "# Guide", sizeBytes: 7 }],
        createdAt: 12,
      } };
      throw new Error(`unexpected method: ${method} ${JSON.stringify(params)}`);
    });
    const onMessage = vi.fn();
    render(<SkillsPage onMessage={onMessage} onOpenConversation={vi.fn()}/>);
    await screen.findByText("周报整理");

    fireEvent.click(screen.getByRole("button", { name: "添加" }));
    fireEvent.click(screen.getByRole("menuitem", { name: /导入 ZIP/ }));

    await waitFor(() => expect(rpc).toHaveBeenCalledWith("skill/import", expect.objectContaining({
      archivePath: "/tmp/weekly.zip",
    })));
    expect(await screen.findByRole("dialog", { name: "导入技能" })).toBeTruthy();
    expect(onMessage).toHaveBeenCalledWith("技能“导入技能”已安装。");
  });

  test("启停使用服务端返回的新版本刷新页面", async () => {
    rpc.mockImplementation(async (method: string, _params: Record<string, unknown>) => {
      if (method === "skill/list") return { skills: [userSkill] };
      if (method === "skill/setState") {
        return { skill: { ...userSkill, enabled: false, stateRevision: 4, updatedAt: 11 } };
      }
      throw new Error(`unexpected method: ${method}`);
    });
    render(<SkillsPage onMessage={vi.fn()} onOpenConversation={vi.fn()}/>);
    const toggle = await screen.findByRole("button", { name: "周报整理已启用" });
    fireEvent.click(toggle);

    await waitFor(() => expect(screen.getByRole("button", { name: "周报整理已停用" })).toBeTruthy());
    expect(rpc).toHaveBeenCalledWith("skill/setState", expect.objectContaining({
      skillId: userSkill.skillId,
      enabled: false,
      automatic: false,
      expectedRevision: 3,
    }));
  });
});
