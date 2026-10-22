import { WindowsSandboxProvider } from "./use-windows-sandbox";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ComponentProps } from "react";
import { AiSettingsPage as AiSettingsPageImplementation } from "./ai-settings";
import { aiErrorMessage } from "./ai-runtime";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ listen: async () => () => {}, invoke }));
vi.mock("./model-connection-settings", () => ({ ModelConnectionSettings: () => <section aria-label="个人模型连接"/> }));
function AiSettingsPage(props: Omit<ComponentProps<typeof AiSettingsPageImplementation>, "hostPlatform"> & { hostPlatform?: string | null }) {
  const { hostPlatform = "macos", ...pageProps } = props;
  return <WindowsSandboxProvider scopeKey="test-profile"><AiSettingsPageImplementation {...pageProps} hostPlatform={hostPlatform}/></WindowsSandboxProvider>;
}

afterEach(cleanup);

describe("AiSettingsPage 模型设置", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockImplementation((command: string) => {
      if (command === "powershell_policy_check") return Promise.resolve({status:"configured",engines:[],changes:[],restoreChanges:[]});
      return Promise.reject(new Error(`unexpected command ${command}`));
    });
  });

  /*
   * 宿主经 IPC 抛回的错误裹着 "Error invoking remote method '...': Error: " 一层壳。此前只剥
   * 开头的 "Error: "，裹过的剥不掉，末尾那张九十多条的映射表于是一条都命中不了——界面上始终
   * 是整串英文原文，而映射表看起来又是写好的，没人会想到它从未生效。
   */
  it("宿主 IPC 包装过的错误码照样映射成中文", () => {
    const wrapped = "Error invoking remote method 'host:invoke': Error: conversation_run_already_active";
    expect(aiErrorMessage(new Error(wrapped))).toBe("当前对话已有运行正在执行，请等待完成或先终止。");
    // 裸错误码与只裹一层 Error: 的形状同样要认
    expect(aiErrorMessage(new Error("conversation_run_already_active"))).toBe("当前对话已有运行正在执行，请等待完成或先终止。");
    expect(aiErrorMessage("conversation_run_already_active")).toBe("当前对话已有运行正在执行，请等待完成或先终止。");
    expect(aiErrorMessage(new Error("thread_binding_invalid"))).toBe("对话绑定的模型线程记录无效，已暂停发送。原记录已保留。");
    expect(aiErrorMessage(new Error("application_skill_not_installed"))).toBe("当前应用所需的 Skill 尚未安装或未启用，请在技能管理中安装并启用后重试。");
    expect(aiErrorMessage(new Error("application_skill_version_mismatch"))).toBe("当前应用所需的 Skill 版本不匹配，请在技能管理中更新后重试。");
    expect(aiErrorMessage(new Error("application_skill_snapshot_unavailable"))).toBe("当前应用所需的 Skill 无法加载，请在技能管理中检查后重试。");
    expect(aiErrorMessage(new Error("Error invoking remote method 'host:invoke': Error: codex_harness_turn_not_active"))).toBe("当前回复无法接收补充。");
    // 带前缀匹配的那一类也要能穿过外壳
    expect(aiErrorMessage(new Error(`Error invoking remote method 'host:invoke': Error: runtime_action_timed_out:安装`)))
      .toBe("工具操作超过 2 分钟，已自动终止。请检查网络后重试。");
    // 认不出来的错误仍然原样透出，不吞掉线索
    expect(aiErrorMessage(new Error("something_unmapped"))).toBe("something_unmapped");
  });

  it("Windows 沙箱初始化失败不向界面透传内部错误串", () => {
    const message = aiErrorMessage(new Error("windows_sandbox_setup_failed:rpc failed:C:\\private\\sandbox.json"));
    expect(message).toBe("Windows 安全沙箱初始化失败，请检查 UAC、组策略，或联系 IT 管理员。");
    expect(message).not.toContain("sandbox.json");
    expect(aiErrorMessage(new Error("ai_full_access_unsupported_platform"))).toBe(
      "当前平台不支持本地模型完全访问权限。",
    );
  });

  it("Windows 检查发现沙箱未初始化时自动发起一次提升权限初始化", async () => {
    const previous = invoke.getMockImplementation()!;
    invoke.mockImplementation((command: string, ...args: unknown[]) => {
      if (command === "ai_windows_sandbox_status") return Promise.resolve({ supported: true, status: "notConfigured", errorCode: null, errorMessage: null });
      if (command === "ai_windows_sandbox_setup") return Promise.resolve({ supported: true, status: "ready", errorCode: null, errorMessage: null });
      return previous(command, ...args);
    });

    render(<AiSettingsPage hostPlatform="windows" workspace={null} onMessage={() => {}}/>);

    const sandbox = await screen.findByRole("region", { name: "Windows 安全沙箱" });
    await waitFor(() => expect(within(sandbox).getByText("已初始化")).toBeTruthy());
    expect(invoke.mock.calls.filter(([command]) => command === "ai_windows_sandbox_setup")).toEqual([["ai_windows_sandbox_setup", { workspaceRoot: null }]]);
  });

  it("Windows 沙箱复查失败后不保留旧的就绪状态", async () => {
    const previous = invoke.getMockImplementation()!;
    let checks = 0;
    invoke.mockImplementation((command: string, ...args: unknown[]) => {
      if (command === "ai_windows_sandbox_status") {
        checks += 1;
        return checks === 1
          ? Promise.resolve({ supported: true, status: "ready", errorCode: null, errorMessage: null })
          : Promise.reject(new Error("windows_sandbox_status_failed:connection closed"));
      }
      return previous(command, ...args);
    });

    render(<AiSettingsPage hostPlatform="windows" workspace={null} onMessage={() => {}}/>);

    const sandbox = await screen.findByRole("region", { name: "Windows 安全沙箱" });
    await waitFor(() => expect(within(sandbox).getByText("已初始化")).toBeTruthy());
    fireEvent.click(within(sandbox).getByRole("button", { name: "重新检查" }));

    await waitFor(() => expect(within(sandbox).getByText("不可用")).toBeTruthy());
    expect(sandbox.getAttribute("data-ready")).not.toBe("true");
    expect(within(sandbox).getByRole("alert").textContent).toContain("Windows 安全沙箱不可用");
  });

});
