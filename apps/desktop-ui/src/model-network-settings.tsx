import { forwardRef, useImperativeHandle, useRef, useState, type Ref } from "react";
import type { ModelConnectionConfig, ModelCredentialAction, ModelNetworkConfig } from "./generated/protocol";
import { Select } from "./select";
import { hasWorkbenchCredentialLocation } from "./credential-location";

export type ModelNetworkSecret = {
  proxyBasic: { username: string; password: string } | null;
  serviceBasic: { username: string; password: string } | null;
  headers: Record<string, string>;
};
export type NetworkCredentialsHandle = { take: () => ModelNetworkSecret };
export type NetworkSave = { networkCredentialAction: ModelCredentialAction; networkSecret?: ModelNetworkSecret };
export type NetworkSaveHandle = { take: () => NetworkSave };
const requiresCredentials = (network: ModelNetworkConfig) => network.proxyAuth === "basic" || network.serviceAuth === "basic" || network.headerNames.length > 0;
const signature = (config: ModelConnectionConfig) => JSON.stringify([config.baseUrl, config.network.mode, config.network.proxyUrl,
  config.network.proxyAuth, config.network.serviceAuth, config.network.headerNames]);

export const NetworkCredentialsInputs = forwardRef<NetworkCredentialsHandle, { network: ModelNetworkConfig; disabled?: boolean }>(function NetworkCredentialsInputs({ network, disabled }, ref) {
  const inputs = useRef<Record<string, HTMLInputElement | null>>({});
  useImperativeHandle(ref, () => ({ take: () => {
    const value = (name: string) => inputs.current[name]?.value ?? "";
    const secret: ModelNetworkSecret = {
      proxyBasic: network.proxyAuth === "basic" ? { username: value("proxy-user"), password: value("proxy-password") } : null,
      serviceBasic: network.serviceAuth === "basic" ? { username: value("service-user"), password: value("service-password") } : null,
      headers: Object.fromEntries(network.headerNames.map((name) => [name, value(`header-${name}`)])),
    };
    for (const input of Object.values(inputs.current)) if (input) input.value = "";
    return secret;
  } }), [network]);
  const field = (name: string, label: string) => <label key={name}>{label}<input ref={(node) => { inputs.current[name] = node; }}
    type="password" required disabled={disabled} autoComplete="new-password" spellCheck={false} maxLength={8_192}/></label>;
  return <div className="personal-model-network-credentials">
    {network.proxyAuth === "basic" ? <>{field("proxy-user", "代理用户")}{field("proxy-password", "代理密码")}</> : null}
    {network.serviceAuth === "basic" ? <>{field("service-user", "服务用户")}{field("service-password", "服务密码")}</> : null}
    {network.headerNames.map((name) => field(`header-${name}`, `请求头值：${name}`))}
  </div>;
});

export function ModelNetworkSettings({ config, original, disabled, onChange, inputRef }: {
  config: ModelConnectionConfig; original: ModelConnectionConfig | null; disabled: boolean;
  onChange: (config: ModelConnectionConfig) => void; inputRef: Ref<NetworkSaveHandle>;
}) {
  const [replace, setReplace] = useState(false);
  const [headerText, setHeaderText] = useState(config.network.headerNames.join("\n"));
  const credentials = useRef<NetworkCredentialsHandle>(null);
  const network = config.network;
  const required = requiresCredentials(network);
  const localReference = hasWorkbenchCredentialLocation(original?.network.proxySecretRef, "model-network");
  const legacyReference = Boolean(original?.network.proxySecretRef) && !localReference;
  const changed = !original || !localReference || signature(config) !== signature(original);
  const replacing = changed || replace;
  useImperativeHandle(inputRef, () => ({ take: () => {
    if (!required) return { networkCredentialAction: original?.network.proxySecretRef ? "clear" : "keep" };
    if (!replacing) return { networkCredentialAction: "keep" };
    return { networkCredentialAction: "replace", networkSecret: credentials.current?.take() };
  } }), [original, required, replacing]);
  const update = (next: Partial<ModelNetworkConfig>) => onChange({ ...config, network: { ...network, ...next } });
  return <>
    {network.mode !== "direct" ? <label>代理认证<Select ariaLabel="代理认证" disabled={disabled} value={network.proxyAuth}
      onChange={(value) => update({ proxyAuth: value as "none" | "basic" })} options={[{ value: "none", label: "无认证" }, { value: "basic", label: "Basic" }]}/></label> : null}
    {(config.kind === "custom_responses" || config.kind === "local") && config.authMode === "none" ? <label>服务认证<Select ariaLabel="服务认证" disabled={disabled} value={network.serviceAuth}
      onChange={(value) => update({ serviceAuth: value as "none" | "basic" })} options={[{ value: "none", label: "无认证" }, { value: "basic", label: "Basic" }]}/></label> : null}
    {config.kind !== "chatgpt" ? <label>附加请求头名称<textarea rows={2} autoComplete="off" spellCheck={false} disabled={disabled}
      placeholder="每行一个小写名称" value={headerText} onChange={(event) => { setHeaderText(event.target.value); update({ headerNames: event.target.value.split("\n").map((name) => name.trim()).filter(Boolean) }); }}/></label> : null}
    {required && original?.network.proxySecretRef && !changed ? <label className="personal-model-check"><input type="checkbox" disabled={disabled} checked={replace} onChange={(event) => setReplace(event.target.checked)}/>更换网络凭据</label> : null}
    {required && legacyReference ? <p className="settings-footnote">需重新保存到工作台</p> : null}
    {required && replacing ? <NetworkCredentialsInputs key={signature(config)} ref={credentials} network={network} disabled={disabled}/> : required ? <p className="settings-footnote">网络本地凭据已登记</p> : null}
    {required && replacing ? <p className="settings-footnote">网络凭据保存到工作台本地加密凭据库。</p> : null}
  </>;
}
