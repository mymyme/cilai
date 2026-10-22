import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import type { SkillRunInstallation } from "./generated/protocol";
const rpc = vi.hoisted(() => vi.fn());
vi.mock("./app-server-rpc", () => ({ rpc }));
import { SkillInstallationStatus, useSkillInstallations } from "./skill-installations";

const installed: SkillRunInstallation = {
  runId: "run", messageId: "reply", status: "installed", skillId: "skill", name: "图片生成",
  errorReason: null, errorDetail: null, retryable: false, displayContent: "交付完成。",
};
afterEach(() => { cleanup(); vi.clearAllMocks(); });

describe("技能交付状态", () => {
  test("成功状态可查看和试用，不再次保存", () => {
    const view = vi.fn(); const trial = vi.fn(); const retry = vi.fn();
    render(<SkillInstallationStatus installation={installed} onRetry={retry} onView={view} onTry={trial}/>);
    expect(screen.getByText("已添加 · 图片生成")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "查看" }));
    fireEvent.click(screen.getByRole("button", { name: "试用" }));
    expect(view).toHaveBeenCalledWith("skill");
    expect(trial).toHaveBeenCalledWith("skill", "图片生成");
    expect(retry).not.toHaveBeenCalled();
  });

  test("内容无效时展示具体原因且不提供无效重试", () => {
    render(<SkillInstallationStatus installation={{ ...installed, status: "invalid", skillId: null,
      errorReason: "skill_delivery_incomplete", errorDetail: "技能内容未完整输出。" }} onRetry={vi.fn()} onTry={vi.fn()}/>);
    expect(screen.getByText("添加失败")).toBeTruthy();
    expect(screen.getByText("技能内容未完整输出。")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "重试" })).toBeNull();
  });

  test("保存失败可以重试，请求失败保留明确原因", async () => {
    const retry = vi.fn().mockRejectedValue(new Error('{"reason":"skill_file_read_failed"}'));
    render(<SkillInstallationStatus installation={{ ...installed, status: "failed", skillId: null,
      retryable: true, errorReason: "skill_file_read_failed" }} onRetry={retry} onTry={vi.fn()}/>);
    fireEvent.click(screen.getByRole("button", { name: "重试" }));
    await waitFor(() => expect(retry).toHaveBeenCalledTimes(1));
    expect(screen.getByText(/无法读取生成的技能文件/)).toBeTruthy();
    expect(screen.queryByText("已添加 · 图片生成")).toBeNull();
  });

  test("历史交付需主动添加，状态只从服务读取且忽略旧缓存", async () => {
    localStorage.setItem("cilai.workbench.ai-skill-install-states", '{"run":"installed"}');
    rpc.mockResolvedValue({ installations: [{ ...installed, status: "historical", skillId: null, retryable: true }] });
    function View() {
      const { installations } = useSkillInstallations(["run"], "profile");
      return <span>{installations.run?.status ?? "loading"}</span>;
    }
    render(<View/>);
    await waitFor(() => expect(screen.getByText("historical")).toBeTruthy());
    expect(rpc.mock.calls.every(([method]) => method === "skill/runList")).toBe(true);
    cleanup(); render(<View/>);
    await waitFor(() => expect(screen.getByText("historical")).toBeTruthy());
    expect(rpc).toHaveBeenCalledTimes(2);
  });

  test("离开页面后收到旧请求不会污染另一个Profile", async () => {
    let resolveOld!: (value: unknown) => void;
    rpc.mockImplementationOnce(() => new Promise((resolve) => { resolveOld = resolve; }))
      .mockResolvedValue({ installations: [] });
    function View({ profile }: { profile: string }) {
      const { installations } = useSkillInstallations(["run"], profile);
      return <span>{installations.run?.name ?? "empty"}</span>;
    }
    const rendered = render(<View profile="first"/>);
    rendered.rerender(<View profile="second"/>);
    await act(async () => { resolveOld({ installations: [installed] }); });
    expect(screen.queryByText("图片生成")).toBeNull();
    expect(screen.getByText("empty")).toBeTruthy();
  });
});
