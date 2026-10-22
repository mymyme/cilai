import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { PersonalPackageImport } from "./personal-package-import";
import type { PersonalPackageImportPreview, PersonalPackageImportTarget } from "./generated/protocol";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
const profileId = "a".repeat(32);
const target: PersonalPackageImportTarget = { mode: "new" };
const original: PersonalPackageImportPreview = {
  importId: "import-one", profileId, state: "prepared", preparedAt: 1, updatedAt: 1, previewDigest: "preview-digest",
  source: { authenticated: false, formatVersion: 1, archiveDigest: `sha256:${"b".repeat(64)}`, sourceDigest: `sha256:${"c".repeat(64)}`,
    pluginId: "external-unverified-package", version: "2.5.0", title: "外来检查器", summary: "来源提供的摘要",
    targetType: "web_mcp_application", sourceFileCount: 2, archiveBytes: 100, sourceBytes: 180, unpackedBytes: 200,
    fullBusinessGoalPresent: false, acceptanceExamplesPresent: false },
  target, localCreationProjectId: `capability_${"d".repeat(32)}`, expectedRevision: null, installedDigest: null,
  localVersion: "0.1.0", localPackageDigest: `sha256:${"e".repeat(64)}`,
  requestedPermissions: ["personal_file.read"], addedPermissions: ["personal_file.read"],
};
let preview: PersonalPackageImportPreview;
let records: PersonalPackageImportPreview[];
const mutations = () => invoke.mock.calls.filter(([command]) => ["personal_package_import_prepare", "personal_package_import_commit", "personal_package_import_cancel"].includes(command));
beforeEach(() => {
  preview = structuredClone(original); records = [];
  invoke.mockReset().mockImplementation(async (command: string) => {
    if (command === "personal_package_import_list") return structuredClone(records);
    if (command === "personal_package_import_prepare" || command === "personal_package_import_get") return structuredClone(preview);
    if (command === "personal_package_import_commit") return { importId: preview.importId, localPackageDigest: preview.localPackageDigest,
      project: { creationProjectId: preview.localCreationProjectId }, replayed: false };
    if (command === "personal_package_import_cancel") return { ...preview, state: "cancelled" };
    throw new Error(`unexpected command ${command}`);
  });
});
afterEach(cleanup);
function open(props: Partial<React.ComponentProps<typeof PersonalPackageImport>> = {}) {
  const onChanged = vi.fn(async () => {}), onClose = vi.fn();
  render(<PersonalPackageImport target={target} profileId={profileId} onChanged={onChanged} onClose={onClose} {...props}/>);
  return { onChanged, onClose };
}
async function choose() {
  const button = screen.getByRole("button", { name: "选择安装包" });
  await waitFor(() => expect(button).toHaveProperty("disabled", false));
  fireEvent.click(button);
  await screen.findByRole("region", { name: "安装包预检" });
  await waitFor(() => expect(screen.getByRole("checkbox", { name: "我已核对来源、版本和权限" })).toHaveProperty("disabled", false));
}
function confirm() {
  fireEvent.click(screen.getByRole("checkbox", { name: "我已核对来源、版本和权限" }));
  fireEvent.click(screen.getByRole("button", { name: "确认安装" }));
}

it("打开仅查询本地未完成记录，不选择文件或重发安装", async () => {
  records = [preview]; open();
  fireEvent.click(await screen.findByRole("button", { name: /外来检查器/ }));
  await screen.findByText("来源未认证");
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(["personal_package_import_list", "personal_package_import_get"]);
  expect(screen.getByRole("button", { name: "确认安装" })).toHaveProperty("disabled", true);
});

it("取消原生选择不产生准备成功、安装或取消回执", async () => {
  open();
  await waitFor(() => expect(screen.getByRole("button", { name: "选择安装包" })).toHaveProperty("disabled", false));
  invoke.mockResolvedValueOnce(null); fireEvent.click(screen.getByRole("button", { name: "选择安装包" }));
  await waitFor(() => expect(mutations()).toHaveLength(1));
  expect(screen.queryByRole("region", { name: "安装包预检" })).toBeNull();
  expect(mutations()[0][0]).toBe("personal_package_import_prepare");
});

it("新建预检保留来源版本、未知目标及权限，关闭保留未完成记录", async () => {
  const { onClose } = open(); await choose();
  expect(screen.getByText("2.5.0")).toBeTruthy(); expect(screen.getByText("0.1.0")).toBeTruthy();
  expect(screen.getByText("来源包未包含完整目标或验收样例。")).toBeTruthy();
  expect(screen.getByText("读取选定文件（新增）")).toBeTruthy();
  expect(screen.getByRole("button", { name: "确认安装" })).toHaveProperty("disabled", true);
  expect(invoke).toHaveBeenCalledWith("personal_package_import_prepare", { request: { target, clientOperationId: expect.any(String) } });
  fireEvent.click(screen.getByRole("button", { name: "关闭" })); expect(onClose).toHaveBeenCalledOnce();
  expect(mutations().map(([command]) => command)).toEqual(["personal_package_import_prepare"]);
});

it("明确确认后只提交所见摘要与权限，成功刷新所属应用", async () => {
  const { onChanged } = open(); await choose(); confirm();
  await screen.findByText("导入已确认");
  expect(invoke).toHaveBeenCalledWith("personal_package_import_commit", { request: {
    importId: preview.importId, previewDigest: preview.previewDigest, confirmedPermissions: preview.requestedPermissions,
    clientOperationId: `personal-import-${preview.importId}`,
  } });
  expect(onChanged).toHaveBeenCalledWith(preview.localCreationProjectId);
  expect(screen.queryByRole("button", { name: "确认安装" })).toBeNull();
});

it("提交结果未知时不重做，先只读核对已提交状态", async () => {
  const { onChanged } = open(); await choose();
  invoke.mockRejectedValueOnce(new Error("结果未知")); confirm();
  await screen.findByText("结果尚未确认，请先核对记录。");
  expect(screen.getByRole("button", { name: "确认安装" })).toHaveProperty("disabled", true);
  preview.state = "committed"; fireEvent.click(screen.getByRole("button", { name: "核对" }));
  await screen.findByText("已安装");
  expect(mutations().filter(([command]) => command === "personal_package_import_commit")).toHaveLength(1);
  expect(onChanged).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "刷新应用" }));
  await waitFor(() => expect(onChanged).toHaveBeenCalledOnce());
});

it("取消导入提交精确标识，失败时必须核对而非伪报取消", async () => {
  open(); await choose();
  invoke.mockRejectedValueOnce(new Error("未收到回执")); fireEvent.click(screen.getByRole("button", { name: "取消导入" }));
  await screen.findByText("结果尚未确认，请先核对记录。");
  expect(screen.queryByText("已取消导入")).toBeNull();
  expect(invoke).toHaveBeenCalledWith("personal_package_import_cancel", { request: { importId: preview.importId, previewDigest: preview.previewDigest } });
  expect(mutations()).toHaveLength(2);
});

it("取消成功后保留可观察终态且不安装", async () => {
  open(); await choose(); fireEvent.click(screen.getByRole("button", { name: "取消导入" }));
  await screen.findByText("已取消导入");
  expect(screen.queryByRole("button", { name: "确认安装" })).toBeNull();
  expect(mutations().some(([command]) => command === "personal_package_import_commit")).toBe(false);
});

it("来源准备不能把明确升级请求改成新建或另一目标", async () => {
  const upgrade: PersonalPackageImportTarget = { mode: "upgrade", creationProjectId: `capability_${"f".repeat(32)}`, expectedRevision: 4 };
  open({ target: upgrade, targetTitle: "当前应用" });
  await waitFor(() => expect(screen.getByRole("button", { name: "选择安装包" })).toHaveProperty("disabled", false));
  fireEvent.click(screen.getByRole("button", { name: "选择安装包" }));
  await screen.findByText("导入记录无法核对，操作已停止。请刷新记录。");
  expect(screen.queryByRole("button", { name: "确认安装" })).toBeNull();
  expect(invoke).toHaveBeenCalledWith("personal_package_import_prepare", { request: { target: upgrade, clientOperationId: expect.any(String) } });
});

it.each(["profile", "permission"])("预检归属或权限不匹配时不能进入确认：%s", async (invalid) => {
  records = [invalid === "profile" ? { ...preview, profileId: "other-profile" } : { ...preview, requestedPermissions: ["constructor"] }]; open();
  await screen.findByText("导入记录无法核对，操作已停止。请刷新记录。");
  expect(screen.queryByRole("button", { name: /外来检查器/ })).toBeNull();
  expect(mutations()).toHaveLength(0);
});

it("安装已提交而列表刷新失败时只重试列表读取", async () => {
  const onChanged = vi.fn().mockRejectedValueOnce(new Error("列表读取失败")).mockResolvedValueOnce(undefined);
  open({ onChanged }); await choose(); confirm();
  await screen.findByText("导入已确认，应用列表尚未刷新。");
  fireEvent.click(screen.getByRole("button", { name: "刷新应用" }));
  await screen.findByText("应用列表已刷新");
  expect(onChanged).toHaveBeenCalledTimes(2);
  expect(mutations().filter(([command]) => command === "personal_package_import_commit")).toHaveLength(1);
});

it("返回另一目标的提交回执不能显示导入完成", async () => {
  const { onChanged } = open(); await choose();
  invoke.mockResolvedValueOnce({ importId: preview.importId, localPackageDigest: preview.localPackageDigest,
    project: { creationProjectId: "another-target" }, replayed: false });
  confirm();
  await screen.findByText("导入记录无法核对，操作已停止。请刷新记录。");
  expect(onChanged).not.toHaveBeenCalled(); expect(screen.queryByText("导入已确认")).toBeNull();
});

it("同名页面权限下的声明变更必须单独核对，提交仍绑定原预检摘要", async () => {
  const pageAi = { schemaVersion: 1, contextTypes: [{ contextType: "note.current", schemaVersion: "1.0", dataSchema: { type: "object", properties: { text: { type: "string", maxLength: 200 } }, additionalProperties: false } }], uiActions: [], model: true, conversationHandoff: false, readTools: [], writeTools: [] };
  preview.source.pageAi = pageAi; preview.installedPageAi = structuredClone(pageAi);
  preview.installedPageAi.contextTypes[0].dataSchema = { type: "object", properties: { text: { type: "string", maxLength: 100 } }, additionalProperties: false };
  preview.requestedPermissions = ["page_ai.context", "page_ai.model"]; preview.addedPermissions = [];
  open(); await choose();
  expect(screen.getByText("声明有变化")).toBeTruthy();
  expect(screen.getByRole("region", { name: "已安装" })).toBeTruthy();
  fireEvent.click(screen.getByRole("checkbox", { name: "我已核对来源、版本和权限" }));
  expect(screen.getByRole("button", { name: "确认安装" })).toHaveProperty("disabled", true);
  expect(mutations().some(([command]) => command === "personal_package_import_commit")).toBe(false);
  fireEvent.click(screen.getByRole("checkbox", { name: "我已核对页面 AI 声明变更" }));
  fireEvent.click(screen.getByRole("button", { name: "确认安装" }));
  await screen.findByText("导入已确认");
  expect(invoke).toHaveBeenCalledWith("personal_package_import_commit", { request: {
    importId: preview.importId, previewDigest: preview.previewDigest, confirmedPermissions: ["page_ai.context", "page_ai.model"], clientOperationId: `personal-import-${preview.importId}`,
  } });
});

it("纯Web页面模型包可明确导入，文件权限与本地MCP页面权限不能混入", async () => {
  preview.source.targetType = "web_application";
  preview.source.pageAi = { schemaVersion: 1, contextTypes: [], uiActions: [], model: true, conversationHandoff: false, readTools: [], writeTools: [] };
  preview.requestedPermissions = ["page_ai.model"]; preview.addedPermissions = ["page_ai.model"];
  open(); await choose();
  expect(screen.getByText("页面模型（新增）")).toBeTruthy();
  expect(screen.getByRole("checkbox", { name: "我已核对页面 AI 声明变更" })).toHaveProperty("checked", false);
  cleanup(); preview.requestedPermissions.push("personal_file.read"); records = [preview]; open();
  await screen.findByText("导入记录无法核对，操作已停止。请刷新记录。");
  expect(screen.queryByRole("region", { name: "安装包预检" })).toBeNull();
  cleanup(); preview.source.targetType = "local_mcp"; preview.requestedPermissions = ["page_ai.model"]; records = [preview]; open();
  await screen.findByText("导入记录无法核对，操作已停止。请刷新记录。");
  expect(screen.queryByRole("button", { name: "确认安装" })).toBeNull();
});

it("页面AI声明读取畸形时不展示成功预检或提交安装", async () => {
  preview.source.pageAi = { schemaVersion: 1, contextTypes: [], uiActions: [], model: true, conversationHandoff: false, readTools: [], writeTools: [] };
  (preview.source.pageAi as unknown as Record<string, unknown>).uiActions = null;
  preview.requestedPermissions = ["page_ai.model"]; records = [preview]; open();
  await screen.findByText("导入记录无法核对，操作已停止。请刷新记录。");
  expect(mutations()).toHaveLength(0);
});
