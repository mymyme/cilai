import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { PersonalWebApplication } from "./personal-web-application";

const host = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("./host", () => host);
const props = { projectId: "personal-counter", packageDigest: `sha256:${"a".repeat(64)}`, html: "<button>加一</button><script>let count = 0;</script>", title: "计数器", active: true };
beforeEach(() => { host.invoke.mockReset(); host.invoke.mockResolvedValue("app://personal-capability/counter"); });
afterEach(cleanup);

test("安装后的纯Web加载独立来源，仅执行页面代码，不接入工具或文件桥接", async () => {
  const view = render(<PersonalWebApplication {...props}/>);
  const frame = await screen.findByTitle("计数器") as HTMLIFrameElement;
  expect(frame.getAttribute("sandbox")).toBe("allow-scripts");
  expect(frame.src).toBe("app://personal-capability/counter");
  expect(frame.srcdoc).toBe("");
  expect(host.invoke).toHaveBeenCalledExactlyOnceWith("personal_document_create", { html: expect.stringContaining(props.html), pageBinding: { creationProjectId: props.projectId, expectedPackageDigest: props.packageDigest } });
  const source = host.invoke.mock.calls[0][1].html;
  expect(source).toContain("ai:window.__cilaiPersonalAi");
  expect(source).not.toContain("listTools:");
  for (const method of ["tools/list", "tools/call", "files/select", "files/write"]) {
    act(() => window.dispatchEvent(new MessageEvent("message", { source: frame.contentWindow, data: { type: "cilai-personal-request", channel: "forged", id: "1", method, params: {} } })));
  }
  expect(host.invoke).toHaveBeenCalledTimes(1);
  view.rerender(<PersonalWebApplication {...props} active={false}/>);
  expect(frame.hidden).toBe(true);
  view.rerender(<PersonalWebApplication {...props}/>);
  expect(screen.getByTitle("计数器")).toBe(frame);
  expect(frame.hidden).toBe(false);
  view.unmount();
  expect(host.invoke).toHaveBeenLastCalledWith("personal_document_release", { url: frame.src });
});

test("页面创建失败明确显示错误，关闭中完成的创建立即释放", async () => {
  host.invoke.mockRejectedValue(new Error("应用页面超过加载限制"));
  render(<PersonalWebApplication {...props}/>);
  expect((await screen.findByRole("alert")).textContent).toContain("应用页面超过加载限制");
  expect(screen.queryByTitle("计数器")).toBeNull();
  cleanup();
  let resolve!: (url: string) => void;
  host.invoke.mockImplementation(async command => command === "personal_document_create" ? new Promise<string>(done => { resolve = done; }) : undefined);
  const view = render(<PersonalWebApplication {...props}/>);
  await waitFor(() => expect(resolve).toBeTypeOf("function"));
  view.unmount();
  await act(async () => resolve("app://personal-capability/late"));
  expect(host.invoke).toHaveBeenLastCalledWith("personal_document_release", { url: "app://personal-capability/late" });
});
