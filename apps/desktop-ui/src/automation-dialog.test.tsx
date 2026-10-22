import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { AutomationDialog, minutesToTime, timeToMinutes, type AutomationDraft } from "./automation-dialog";
import type { AiRuntimeSnapshot } from "./ai-runtime";
import { scheduleLabel } from "./automations";

afterEach(cleanup);

const draft: AutomationDraft = {
  automationRuleId: null,
  name: "周五日报",
  actionPrompt: "汇总本周变更并生成周报",
  scheduleKind: "weekly",
  intervalMinutes: 60,
  timeOfDayMinutes: 17 * 60 + 45,
  weekday: 5,
  weekdays: [5],
  notifyOnEveryRun: false,
  agentId: "agent-1",
  modelId: null,
  projectId: "project-1",
  executionMode: "new",
  conversationId: null,
  enabled: true,
};

const agents = [{ agentId: "agent-1", name: "主控" }] as never;

const setup = (over: Partial<AutomationDraft> = {}, conversations: AiRuntimeSnapshot["conversations"] = []) => {
  const onDraft = vi.fn();
  const onSave = vi.fn();
  render(<AutomationDialog agents={agents} models={[]} projects={[{ projectId: "project-1", displayName: "示例项目", primaryWorkspaceId: "ws-1", primaryWorkspaceName: "主工作区", primaryWorkspaceRoot: "/tmp", workspaceIds: ["ws-1"] }]} conversations={conversations} busy={false} draft={{ ...draft, ...over }} onCancel={() => {}} onDraft={onDraft} onSave={onSave}/>);
  return { onDraft, onSave };
};

describe("时间换算", () => {
  it("分钟数转为时间输入格式，补零", () => {
    expect(minutesToTime(0)).toBe("00:00");
    expect(minutesToTime(9 * 60 + 5)).toBe("09:05");
    expect(minutesToTime(17 * 60 + 45)).toBe("17:45");
  });

  it("超出一天的取值被夹到合法范围", () => {
    expect(minutesToTime(-30)).toBe("00:00");
    expect(minutesToTime(5000)).toBe("23:59");
  });

  it("时间输入转回分钟数", () => {
    expect(timeToMinutes("17:45")).toBe(17 * 60 + 45);
    expect(timeToMinutes("00:00")).toBe(0);
  });

  it("非法时间不产生 NaN", () => {
    expect(timeToMinutes("")).toBe(0);
    expect(timeToMinutes("abc")).toBe(0);
  });

  it("换算可往返", () => {
    for (const minutes of [0, 1, 59, 60, 725, 1439]) {
      expect(timeToMinutes(minutesToTime(minutes))).toBe(minutes);
    }
  });
});

describe("调度文案", () => {
  it("按输入的方式表述，而不是统一折算成间隔", () => {
    expect(scheduleLabel({ scheduleKind: "weekly", intervalMinutes: 60, timeOfDayMinutes: 17 * 60 + 45, weekday: 5 })).toBe("每周五 17:45");
    expect(scheduleLabel({ scheduleKind: "daily", intervalMinutes: 60, timeOfDayMinutes: 9 * 60, weekday: 1 })).toBe("每天 09:00");
    expect(scheduleLabel({ scheduleKind: "interval", intervalMinutes: 15, timeOfDayMinutes: 0, weekday: 1 })).toBe("每 15 分钟");
    expect(scheduleLabel({ scheduleKind: "interval", intervalMinutes: 240, timeOfDayMinutes: 0, weekday: 1 })).toBe("每 4 小时");
  });
});

describe("自动化弹窗", () => {
  it("未选项目时不能设置对话方式或保存", () => {
    setup({ projectId: null });
    expect(screen.getByLabelText("对话方式")).toHaveProperty("disabled", true);
    expect(screen.getByRole("button", { name: "保存" })).toHaveProperty("disabled", true);
  });

  it("已有对话模式必须选择可用对话", () => {
    setup({ executionMode: "existing", conversationId: "missing" });
    expect(screen.getByRole("button", { name: "保存" })).toHaveProperty("disabled", true);
    expect(screen.getByLabelText("选择对话")).toHaveProperty("disabled", true);
    expect(screen.getByLabelText("模型")).toBeTruthy();
  });

  it("新建和编辑都能明确选择对话方式", () => {
    const { onDraft } = setup();
    expect(screen.queryByLabelText("选择对话")).toBeNull();
    fireEvent.click(screen.getByLabelText("对话方式"));
    fireEvent.click(screen.getByRole("option", { name: "已有对话" }));
    expect(onDraft).toHaveBeenCalledWith(expect.objectContaining({ executionMode: "existing", conversationId: null }));
    cleanup();
    const edited = setup({ executionMode: "existing", conversationId: "missing" });
    fireEvent.click(screen.getByLabelText("对话方式"));
    fireEvent.click(screen.getByRole("option", { name: "新对话" }));
    expect(edited.onDraft).toHaveBeenCalledWith(expect.objectContaining({ executionMode: "new", conversationId: null }));
  });

  it("已有对话按名称选择，同名对话显示更新时间", () => {
    const conversations = [
      { conversationId: "first", title: "周报", archived: false, updatedAt: 1_790_000_000_000, context: { projectId: "project-1" } },
      { conversationId: "second", title: "周报", archived: false, updatedAt: 1_790_100_000_000, context: { projectId: "project-1" } },
      { conversationId: "other", title: "其他项目", archived: false, updatedAt: 1, context: { projectId: "project-2" } },
      { conversationId: "archived", title: "已归档", archived: true, updatedAt: 1, context: { projectId: "project-1" } },
    ] as AiRuntimeSnapshot["conversations"];
    const { onDraft } = setup({ executionMode: "existing" }, conversations);
    fireEvent.click(screen.getByLabelText("选择对话"));
    expect(screen.getAllByRole("option", { name: /^周报 · / })).toHaveLength(2);
    expect(screen.queryByRole("option", { name: "其他项目" })).toBeNull();
    expect(screen.queryByRole("option", { name: "已归档" })).toBeNull();
    fireEvent.click(screen.getAllByRole("option", { name: /^周报 · / })[1]);
    expect(onDraft).toHaveBeenCalledWith(expect.objectContaining({ conversationId: "second" }));
  });
  it("指令是弹窗的主体，可直接编辑", () => {
    const { onDraft } = setup();
    const prompt = screen.getByLabelText("交给智能体执行的指令");
    expect((prompt as HTMLTextAreaElement).value).toBe("汇总本周变更并生成周报");
    fireEvent.change(prompt, { target: { value: "改后的指令" } });
    expect(onDraft).toHaveBeenCalledWith(expect.objectContaining({ actionPrompt: "改后的指令" }));
  });

  it("每周才显示星期选择，按间隔时不显示", () => {
    setup();
    expect(screen.queryByRole("group", { name: "每周执行日期" })).not.toBeNull();
    cleanup();
    setup({ scheduleKind: "interval" });
    expect(screen.queryByRole("group", { name: "每周执行日期" })).toBeNull();
    expect(screen.queryByLabelText("执行间隔")).not.toBeNull();
  });

  it("每周可以选择多个执行日期", () => {
    const { onDraft } = setup();
    fireEvent.click(screen.getByRole("button", { name: "星期一" }));
    expect(onDraft).toHaveBeenCalledWith(expect.objectContaining({ weekdays: [1, 5] }));
  });

  it("按间隔时不显示具体时间，按天和按周才显示", () => {
    setup({ scheduleKind: "interval" });
    expect(screen.queryByLabelText("执行时间")).toBeNull();
    cleanup();
    setup({ scheduleKind: "daily" });
    expect((screen.getByLabelText("执行时间") as HTMLInputElement).value).toBe("17:45");
  });

  it("指令为空时不能保存", () => {
    setup({ actionPrompt: "   " });
    expect((screen.getByRole("button", { name: "保存" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("名称为空时不能保存", () => {
    setup({ name: "" });
    expect((screen.getByRole("button", { name: "保存" }) as HTMLButtonElement).disabled).toBe(true);
  });
});


it("自动化只提供模型选项，已有本地智能体选择必须重新选模型", () => {
  render(<AutomationDialog agents={agents} models={[]} projects={[]} conversations={[]}
    busy={false} draft={draft} onCancel={() => {}} onDraft={() => {}} onSave={() => {}}/>);
  fireEvent.click(screen.getByLabelText("模型"));
  expect(screen.queryByRole("option", { name: "主控" })).toBeNull();
  expect(screen.getByRole("button", { name: "保存" })).toHaveProperty("disabled", true);
});
