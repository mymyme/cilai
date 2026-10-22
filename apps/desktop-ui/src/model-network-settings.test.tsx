import { useRef, useState } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { ModelNetworkSettings, type NetworkSaveHandle } from "./model-network-settings";
import type { ModelConnectionConfig } from "./generated/protocol";

afterEach(cleanup);
const config = (): ModelConnectionConfig => ({ name: "本机合成连接", kind: "custom_responses", baseUrl: "http://127.0.0.1:11434/v1", localRuntime: null,
  authMode: "none", enabled: true, network: { mode: "custom_proxy", proxyUrl: "http://127.0.0.1:8080", proxySecretRef: null, proxyAuth: "none", serviceAuth: "none", headerNames: [] } });
function Editor({ original = null, saved }: { original?: ModelConnectionConfig | null; saved: (value: unknown) => void }) {
  const [value, setValue] = useState(original ?? config());
  const fields = useRef<NetworkSaveHandle>(null);
  return <form onSubmit={(event) => { event.preventDefault(); saved({ config: value, ...fields.current?.take() }); }}>
    <ModelNetworkSettings config={value} original={original} disabled={false} onChange={setValue} inputRef={fields}/>
    <button type="button" onClick={() => setValue({ ...value, network: { ...value.network, proxyUrl: "http://127.0.0.1:8081" } })}>改代理</button>
    <button type="button" onClick={() => setValue({ ...value, network: { ...value.network, caCertificatePem: "public-ca-only" } })}>改证书</button>
    <button type="submit">保存</button>
  </form>;
}
async function choose(label: string, option: string) {
  fireEvent.click(screen.getByRole("combobox", { name: label }));
  fireEvent.click(await screen.findByRole("option", { name: option }));
}

it("代理、服务与附加头值只进入一次私有凭据输入，提交后清空", async () => {
  const saved = vi.fn(); render(<Editor saved={saved}/>);
  await choose("代理认证", "Basic");
  await choose("服务认证", "Basic");
  fireEvent.change(screen.getByLabelText("附加请求头名称"), { target: { value: "x-service-key\nx-tenant-token\n" } });
  for (const [label, value] of [["代理用户", "proxy-user"], ["代理密码", "proxy-private"], ["服务用户", "service-user"], ["服务密码", "service-private"], ["请求头值：x-service-key", "header-private"], ["请求头值：x-tenant-token", "tenant-private"]]) {
    fireEvent.change(screen.getByLabelText(label), { target: { value } });
  }
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  expect(saved).toHaveBeenCalledTimes(1);
  const receipt = saved.mock.calls[0][0];
  expect(receipt.networkCredentialAction).toBe("replace");
  expect(receipt.networkSecret).toEqual({ proxyBasic: { username: "proxy-user", password: "proxy-private" }, serviceBasic: { username: "service-user", password: "service-private" }, headers: { "x-service-key": "header-private", "x-tenant-token": "tenant-private" } });
  expect(JSON.stringify(receipt.config)).not.toContain("private");
  for (const input of screen.getAllByLabelText(/用户|密码|请求头值/)) expect(input).toHaveProperty("value", "");
  expect(screen.getByLabelText("附加请求头名称")).toHaveProperty("value", "x-service-key\nx-tenant-token\n");
});

it("证书变化保留原引用，代理变化必须重新输入凭据", () => {
  const saved = vi.fn(); const original = config();
  original.network.proxyAuth = "basic";
  original.network.proxySecretRef = { service: "cn.cilai.personal.workbench.vault.model-network", account: "synthetic-reference" };
  render(<Editor original={original} saved={saved}/>);
  expect(screen.queryByLabelText("代理密码")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "改证书" }));
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  expect(saved.mock.calls[0][0].networkCredentialAction).toBe("keep");
  fireEvent.click(screen.getByRole("button", { name: "改代理" }));
  expect(screen.getByLabelText("代理密码")).toHaveProperty("value", "");
  expect(saved).toHaveBeenCalledTimes(1);
});

it("移除声明明确清理引用，API Key 连接没有冲突的服务 Basic 入口", async () => {
  const saved = vi.fn(); const original = config();
  original.authMode = "api_key"; original.network.proxyAuth = "basic";
  original.network.proxySecretRef = { service: "cn.cilai.personal.workbench.vault.model-network", account: "synthetic-reference" };
  render(<Editor original={original} saved={saved}/>);
  expect(screen.queryByRole("combobox", { name: "服务认证" })).toBeNull();
  await choose("代理认证", "无认证");
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  expect(saved.mock.calls[0][0].networkCredentialAction).toBe("clear");
  expect(saved.mock.calls[0][0].networkSecret).toBeUndefined();
});


it.each(["cn.cilai.personal.workbench.proxy", "cn.cilai.personal.workbench.vault.model-network.extra"])("网络旧引用%s要求重新输入，不读取或复用旧值", (service) => {
  const saved = vi.fn(); const original = config();
  original.network.proxyAuth = "basic";
  original.network.proxySecretRef = { service, account: "private-network-reference" };
  render(<Editor original={original} saved={saved}/>);
  expect(screen.getByText("需重新保存到工作台")).toBeTruthy();
  for (const label of ["代理用户", "代理密码"]) {
    expect(screen.getByLabelText(label)).toHaveProperty("value", "");
    expect(screen.getByLabelText(label)).toHaveProperty("required", true);
  }
  expect(screen.queryByRole("checkbox", { name: "更换网络凭据" })).toBeNull();
  expect(screen.queryByText("网络本地凭据已登记")).toBeNull();
  expect(document.body.textContent).not.toContain("private-network-reference");
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  expect(saved).not.toHaveBeenCalled();
  fireEvent.change(screen.getByLabelText("代理用户"), { target: { value: "synthetic-user" } });
  fireEvent.change(screen.getByLabelText("代理密码"), { target: { value: "synthetic-password" } });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  expect(saved.mock.calls[0][0].networkCredentialAction).toBe("replace");
  expect(screen.getByLabelText("代理密码")).toHaveProperty("value", "");
  expect(JSON.stringify(saved.mock.calls[0][0].config)).not.toContain("synthetic-password");
});
