import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
const invoke = vi.hoisted(() => vi.fn());
const openPdfDocument = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
vi.mock("./pdfjs-renderer", () => ({ openPdfDocument }));
import { FilePreviewPane } from "./preview";
import { RetainedPage } from "./retained-page";
import type { WorkspaceFileItem } from "./generated/protocol";
const file: WorkspaceFileItem = { localFileId: "file-1", relativePath: "图片.png", fileType: "file", presenceState: "present", displayState: null, baselineRelation: null, localChangeState: null, confirmationState: null, conflictState: null, stateRevision: 2, checksumAlgorithm: "sha256", checksumValue: "digest", baselineVersionId: null };
function pane(kind = "cloud_team", current = file) { return <FilePreviewPane workspaceId="workspace-1" workspaceKind={kind} localRoot={kind === "local" ? "/workspace" : "cilai-cloud://team"} hostPlatform="windows" file={current} view="rendered" onViewAvailabilityChange={() => {}}/>; }
beforeEach(() => {
  invoke.mockReset();
  Object.defineProperty(URL, "createObjectURL", { configurable: true, value: vi.fn(() => "blob:preview-image") });
  Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: vi.fn() });
});
afterEach(() => { cleanup(); vi.clearAllMocks(); });
test("旧云端文件明确不可预览，不请求云服务也不改读同名本地文件", async () => {
  render(pane());
  expect(await screen.findByText("个人版不支持此云端来源，请明确选择本地文件。")).toBeTruthy();
  expect(invoke).not.toHaveBeenCalled();
  expect(screen.queryByRole("img")).toBeNull();
});
test("占位文件实际预览时下载，完成后重建观测再读取", async () => {
  let polls = 0; const order: string[] = [];
  invoke.mockImplementation(async (command, args) => {
    order.push(args?.method ?? command);
    if (args?.method === "file/hydrate") return ++polls === 1 ? { state: "downloading", operationId: "download-1" } : { state: "ready" };
    if (args?.method === "workspace/indexPath") return { localFileId: "file-1" };
    if (command === "preview_open") return { source: "local", format: "image", availability: "ready", relativePath: "图片.png" };
    if (args?.method === "preview/readBinary") return { contentBase64: btoa("image bytes"), contentRevision: "digest", contentType: "image/png", sizeBytes: 11 };
    throw new Error(`unexpected ${command}`);
  });
  render(pane("local"));
  await screen.findByText("正在下载");
  await screen.findByRole("img", { name: "图片.png" });
  expect(order).toEqual(["file/hydrate", "file/hydrate", "workspace/indexPath", "preview_open", "preview/readBinary"]);
});
test("取消下载只取消工作台等待，随后不读取正文", async () => {
  invoke.mockResolvedValue({ state: "downloading", operationId: "download-1" });
  render(pane("local"));
  fireEvent.click(await screen.findByRole("button", { name: "取消" }));
  expect(await screen.findByText("已取消等待下载。")).toBeTruthy();
  expect(invoke).toHaveBeenCalledWith("app_server_request", expect.objectContaining({ method: "file/hydrate", params: expect.objectContaining({ operationId: "download-1", cancel: true }) }));
  expect(invoke.mock.calls.some(([command]) => command === "preview_open")).toBe(false);
});

test("切页保留 PDF 画面和解析资源，切回不重新读取，关闭范围才释放", async () => {
  invoke.mockImplementation(async (command,args) => {
    if (args?.method === "file/hydrate") return {state:"ready"};
    if (command === "preview_open") return {source:"local",format:"pdf",availability:"ready",relativePath:"合同.pdf",contentRevision:"digest"};
    if (command === "workspace_pdf_read_bytes") return new Uint8Array([1,2,3]);
    throw new Error(`unexpected ${command}`);
  });
  const destroy = vi.fn();
  openPdfDocument.mockReturnValue({promise:Promise.resolve({pageCount:1,renderPage:()=>({promise:Promise.resolve(),cancel:vi.fn()})}),destroy});
  const page = (active:boolean) => <RetainedPage active={active}>{pane("local",{...file,relativePath:"合同.pdf"})}</RetainedPage>;
  const view = render(page(true));
  const canvas = await screen.findByRole("img",{name:"PDF 第 1 页"});
  view.rerender(page(false));
  expect(screen.queryByRole("img")).toBeNull();
  expect(destroy).not.toHaveBeenCalled();
  view.rerender(page(true));
  expect(await screen.findByRole("img",{name:"PDF 第 1 页"})).toBe(canvas);
  expect(invoke).toHaveBeenCalledTimes(3);
  expect(openPdfDocument).toHaveBeenCalledTimes(1);
  view.unmount();
  expect(destroy).toHaveBeenCalledTimes(1);
});

test("隐藏页文件实际变更后延迟读取新版本，普通切页不重读", async () => {
  invoke.mockImplementation(async (command,args)=> {
    if (args?.method === "file/hydrate") return {state:"ready"};
    if (command === "preview_open") return {source:"local",format:"image",availability:"ready",relativePath:"图片.png"};
    if (args?.method === "preview/readBinary") return {contentBase64:btoa("bytes"),contentRevision:"digest",contentType:"image/png",sizeBytes:5};
    throw new Error(`unexpected ${command}`);
  });
  const page = (active:boolean,revision=2) => <RetainedPage active={active}>{pane("local",{...file,stateRevision:revision,checksumValue:`digest-${revision}`})}</RetainedPage>;
  const view=render(page(true));
  await screen.findByRole("img");
  const first=invoke.mock.calls.length;
  view.rerender(page(false)); view.rerender(page(true));
  await screen.findByRole("img");
  expect(invoke).toHaveBeenCalledTimes(first);
  view.rerender(page(false,3));
  expect(invoke).toHaveBeenCalledTimes(first);
  view.rerender(page(true,3));
  await screen.findByRole("img");
  expect(invoke.mock.calls.filter(([command])=>command==="preview_open")).toHaveLength(2);
});
