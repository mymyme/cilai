import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, waitFor } from "@testing-library/react";
import { useAgentIncidentReporting } from "./use-agent-incidents";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({
  invoke,
}));

afterEach(cleanup);

const run = (over: Record<string, unknown> = {}) => ({
  runId: "run-1", conversationId: "conv-1", agentId: "agent-1", status: "failed",
  retryOfRunId: "classified-run", startedAt: 1, completedAt: 2,
  errorReason: "provider exited with code 1", ...over,
});

const snapshot = (runs: unknown[], projectId: string | null = "proj-1") => ({
  runtimes: [], agents: [{ agentId: "agent-1", name: "词来 主控" }],
  conversations: [{
    conversationId: "conv-1",
    context: { projectId, phaseId: null, taskId: "task-1" },
    workClassification: { runId: "classified-run", taskId: "task-1" },
  }],
  messages: [],
  runs: [run({ runId: "classified-run", status: "completed", retryOfRunId: null }), ...runs],
  permissionRequests: [], inboxReadItemIds: [],
});

/** 记录发给服务端的 agentIncident/report 请求。 */
function reports() {
  return invoke.mock.calls
    .filter(([command, args]) => command === "app_server_request" && (args as { method: string }).method === "agentIncident/report")
    .map(([, args]) => (args as { params: Record<string, unknown> }).params);
}

function Harness({ root, onError = () => {} }: { root: string | null; onError?: (message: string) => void }) {
  useAgentIncidentReporting({ workspaceRoot: root, onReported: () => {}, onError });
  return null;
}

beforeEach(() => {
  invoke.mockReset();
  invoke.mockImplementation((command: string) => {
    if (command === "ai_runtime_query") return Promise.resolve(snapshot([run()]));
    return Promise.resolve({});
  });
});

describe("失败的运行汇入工作台", () => {
  it("失败的运行上报为关注事项", async () => {
    render(<Harness root="/tmp/ws"/>);
    await waitFor(() => expect(reports()).toHaveLength(1));
    expect(invoke).toHaveBeenCalledWith("ai_runtime_query", { request: { scope: "incidents" } });
    expect(reports()[0].runId).toBe("run-1");
    expect(reports()[0].projectId).toBe("proj-1");
    expect(reports()[0].errorSummary).toContain("provider exited");
  });

  it("优先级取服务端接受的四档之一", async () => {
    // 服务端对取值有硬校验，取值不在四档内会被整条拒绝。而上报失败在前端是静默的，
    // 不在这里钉住，下一次写错同样不会有人发现。
    render(<Harness root="/tmp/ws"/>);
    await waitFor(() => expect(reports()).toHaveLength(1));
    expect(["low", "medium", "high", "critical"]).toContain(reports()[0].priority);
  });

  it("operationId 由 runId 推导，重复上报才能命中幂等而不是撞冲突", async () => {
    render(<Harness root="/tmp/ws"/>);
    await waitFor(() => expect(reports()).toHaveLength(1));
    expect(reports()[0].clientOperationId).toBe("desktop-agent-incident-run-1");
  });

  it("用户自己终止的运行不算事故", async () => {
    invoke.mockImplementation((command: string) =>
      Promise.resolve(command === "ai_runtime_query" ? snapshot([run({ status: "terminated" })]) : {}));
    render(<Harness root="/tmp/ws"/>);
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(reports()).toHaveLength(0);
  });

  it("还活着、只是在等授权的运行不建持久待办卡", async () => {
    invoke.mockImplementation((command: string) =>
      Promise.resolve(command === "ai_runtime_query" ? snapshot([run({ status: "waiting_permission" })]) : {}));
    render(<Harness root="/tmp/ws"/>);
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(reports()).toHaveLength(0);
  });

  it("没有项目归属的对话无处可挂，跳过而不是报错", async () => {
    invoke.mockImplementation((command: string) =>
      Promise.resolve(command === "ai_runtime_query" ? snapshot([run()], null) : {}));
    render(<Harness root="/tmp/ws"/>);
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(reports()).toHaveLength(0);
  });

  it("无关历史失败不按对话当前任务补记阻塞", async () => {
    invoke.mockImplementation((command: string) =>
      Promise.resolve(command === "ai_runtime_query" ? snapshot([run({ retryOfRunId: null })]) : {}));
    render(<Harness root="/tmp/ws"/>);
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(reports()).toHaveLength(0);
  });

  it("补记被服务端拒绝时报出来，不静默吞掉", async () => {
    // 这里原先是一个不分青红皂白的 catch：上报被整条拒绝时前端毫无察觉，
    // 事故卡长期没有建出来过而没有人知道。
    invoke.mockImplementation((command: string, args: unknown) => {
      if (command === "ai_runtime_query") return Promise.resolve(snapshot([run()]));
      if ((args as { method?: string })?.method === "agentIncident/report") {
        return Promise.reject(new Error('{"code":-32010,"data":{"reason":"attention_priority_invalid"}}'));
      }
      return Promise.resolve({});
    });
    const onError = vi.fn();
    render(<Harness root="/tmp/ws" onError={onError}/>);
    await waitFor(() => expect(onError).toHaveBeenCalledWith(expect.stringContaining("未能记入工作台")));
  });

  it("这次运行已经有事故卡时静静跳过，不打扰用户", async () => {
    invoke.mockImplementation((command: string, args: unknown) => {
      if (command === "ai_runtime_query") return Promise.resolve(snapshot([run()]));
      if ((args as { method?: string })?.method === "agentIncident/report") {
        return Promise.reject(new Error('{"code":-32010,"data":{"reason":"agent_run_incident_exists"}}'));
      }
      return Promise.resolve({});
    });
    const onError = vi.fn();
    render(<Harness root="/tmp/ws" onError={onError}/>);
    await waitFor(() => expect(reports()).toHaveLength(1));
    expect(onError).not.toHaveBeenCalled();
  });

  it("没有工作区时不轮询", async () => {
    render(<Harness root={null}/>);
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(invoke).not.toHaveBeenCalled();
  });
});
