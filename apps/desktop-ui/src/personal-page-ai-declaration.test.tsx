import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import type { PersonalPageAiManifest } from "./generated/protocol";
import { isPersonalPageAiManifest, pageAiChanged, pageAiPermissionsMatch, PersonalPageAiDeclaration, permissionsForTarget } from "./personal-page-ai-declaration";

afterEach(cleanup);
export function pageManifest(): PersonalPageAiManifest {
  return {
    schemaVersion: 1,
    contextTypes: [{ contextType: "note.current", schemaVersion: "1.0", dataSchema: { type: "object", properties: { text: { type: "string", maxLength: 100 } }, required: ["text"], additionalProperties: false } }],
    uiActions: [{ actionId: "note.save", title: "保存笔记", description: "保存当前笔记", inputSchema: { type: "object", properties: { text: { type: "string" } }, required: ["text"], additionalProperties: false }, riskLevel: "high", preconditions: ["笔记已打开"], expectedEffects: ["替换笔记内容"], confirmationPolicy: "always" }],
    model: true, conversationHandoff: true,
    readTools: [{ name: "inspect_text", title: "检查文本", description: "返回文本长度", inputSchema: { type: "object", properties: { text: { type: "string" } }, required: ["text"], additionalProperties: false } }],
    writeTools: [{ name: "save_note", title: "写入笔记", description: "写入应用笔记数据", inputSchema: { type: "object", properties: { note: { type: "string" } }, required: ["note"], additionalProperties: false } }],
  };
}
it("完整展示安装前后的上下文、动作确认和精确工具参数", () => {
  const before = pageManifest(), after = pageManifest();
  after.contextTypes[0].dataSchema = { type: "object", properties: { text: { type: "string", maxLength: 200 } }, additionalProperties: false };
  render(<PersonalPageAiDeclaration before={before} after={after}/>);
  expect(screen.getByText("声明有变化")).toBeTruthy();
  const current = within(screen.getByRole("region", { name: "本次版本" }));
  for (const label of ["note.current", "格式版本 1.0", "保存笔记", "note.save", "保存当前笔记", "高", "笔记已打开", "替换笔记内容", "inspect_text", "save_note", "写入应用笔记数据"]) expect(current.getByText(label)).toBeTruthy();
  expect(current.getAllByText("每次确认")).toHaveLength(2);
  expect(current.getAllByText("允许")).toHaveLength(2);
  expect(current.getAllByText(/"maxLength": 200/)).toHaveLength(1);
  expect(within(screen.getByRole("region", { name: "已安装" })).getByText(/"maxLength": 100/)).toBeTruthy();
});
it("相同权限名称下的声明和参数变化仍识别为变化，JSON键序不影响比较", () => {
  const before = pageManifest(), after = structuredClone(before);
  after.contextTypes[0].dataSchema = { additionalProperties: false, required: ["text"], properties: { text: { maxLength: 100, type: "string" } }, type: "object" };
  expect(pageAiChanged(before, after)).toBe(false);
  after.writeTools[0].inputSchema = { type: "object", properties: { path: { type: "string" } }, additionalProperties: false };
  expect(pageAiChanged(before, after)).toBe(true);
  expect(pageAiChanged(undefined, null)).toBe(false);
  expect(pageAiChanged(before, undefined)).toBe(true);
});
it("未声明的旧包无页面授权，纯Web和本地工具保持不同权限边界", () => {
  expect(permissionsForTarget("local_mcp").every(item => !item.page)).toBe(true);
  expect(permissionsForTarget("web_application").every(item => item.page && !item.value.startsWith("page_ai.tools."))).toBe(true);
  expect(pageAiPermissionsMatch("web_application", [], undefined)).toBe(true);
  expect(pageAiPermissionsMatch("web_application", ["page_ai.model"], undefined)).toBe(false);
  expect(pageAiPermissionsMatch("local_mcp", ["page_ai.model"], pageManifest())).toBe(false);
  const full = pageManifest(), grants = permissionsForTarget("web_mcp_application").filter(item => item.page).map(item => item.value);
  expect(pageAiPermissionsMatch("web_mcp_application", grants, full)).toBe(true);
  expect(pageAiPermissionsMatch("web_application", grants, full)).toBe(false);
  expect(pageAiPermissionsMatch("web_mcp_application", grants.slice(1), full)).toBe(false);
  expect(pageAiPermissionsMatch("unknown", [], full)).toBe(false);
});
it("畸形或未确认动作的回执不进入可确认的声明视图", () => {
  expect(isPersonalPageAiManifest({ ...pageManifest(), uiActions: null })).toBe(false);
  expect(isPersonalPageAiManifest({ ...pageManifest(), extra: true })).toBe(false);
  const value = pageManifest() as unknown as { uiActions: { confirmationPolicy: string }[] };
  value.uiActions[0].confirmationPolicy = "automatic";
  expect(isPersonalPageAiManifest(value)).toBe(false);
  render(<PersonalPageAiDeclaration/>);
  expect(screen.queryByRole("region", { name: "页面 AI 声明核对" })).toBeNull();
});
