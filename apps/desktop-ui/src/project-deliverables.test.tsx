import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { ComponentProps } from "react";
import { DeliverableManager } from "./project-deliverables";

const invoke = vi.hoisted(() => vi.fn());
const rpc = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
vi.mock("./app-server-rpc", () => ({ rpc, sha256: async () => "request-fingerprint" }));

type Props = ComponentProps<typeof DeliverableManager>;
const workspace: NonNullable<Props["workspace"]> = {
  workspaceId: "workspace-project", workspaceKind: "local", displayName: "项目文件夹", localRoot: "D:\\项目",
  localStatus: "ready", stateRevision: 1, platformBound: false, platformRevision: null,
  teamSpaceId: null, managedRoot: null, deviceId: null, baselineRevision: null, changeCursor: null,
  permissionRole: null, permissionRevision: null,
};
const project: Props["project"] = {
  projectId: "project-existing", displayName: "已有项目", goal: "形成项目报告", status: "active",
  primaryWorkspaceId: workspace.workspaceId, stateRevision: 3, updatedAt: 1,
};
const deliverable: Props["deliverables"][number] = {
  deliverableId: "deliverable-existing", projectId: project.projectId, title: "项目报告", deliverableType: "document",
  ownerLabel: "项目负责人", acceptanceCriteria: "人工核对原始数据", status: "accepted",
  currentAcceptedVersionId: "version-1", stateRevision: 7, updatedAt: 1,
  versions: [{
    versionId: "version-1", versionNo: 1, status: "accepted", summary: "已有正式报告", createdAt: 1, acceptedAt: 2, rejectedAt: 0,
    bindings: [{ localFileId: "file-report", workspaceId: workspace.workspaceId, relativePath: "报告.md", fileRevision: 4, checksumAlgorithm: "sha256", checksumValue: "original-checksum", bindingRole: "primary" }],
    carriers: [{ carrierId: "carrier-existing", carrierType: "file", carrierRole: "primary", label: "历史产物", reference: "cilai-artifact:run-original/artifact-original", versionLabel: "abcdef1234567890", sourceProjectId: project.projectId, sourceDeliverableId: null, sourceVersionId: null }],
  }],
};
const file: Props["files"][number] = {
  localFileId: "file-report", relativePath: "报告.md", fileType: "file", presenceState: "present", displayState: null,
  baselineRelation: null, localChangeState: null, confirmationState: null, conflictState: null,
  stateRevision: 4, checksumAlgorithm: "sha256", checksumValue: "original-checksum", baselineVersionId: null,
};

function renderManager(overrides: Partial<Props> = {}) {
  const props: Props = {
    hostPlatform: "windows", workspace, project, deliverables: structuredClone([deliverable]), files: [file],
    onClose: vi.fn(), onChanged: vi.fn().mockResolvedValue(undefined), onMessage: vi.fn(), ...overrides,
  };
  render(<DeliverableManager {...props}/>);
  return props;
}

beforeEach(() => { invoke.mockReset().mockResolvedValue(undefined); rpc.mockReset().mockResolvedValue({}); });
afterEach(cleanup);

test.each(["windows", "macos"])("%s 项目已有版本、绑定文件及历史产物仍可查看和打开", async (hostPlatform) => {
  const props = renderManager({ hostPlatform });
  expect(screen.getByRole("heading", { name: "项目报告" })).toBeTruthy();
  expect(screen.getByText("人工核对原始数据")).toBeTruthy();
  expect(screen.getByText("V1 · 已验收")).toBeTruthy();
  expect(screen.getByText("已有正式报告")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "历史产物 · abcdef123456" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_artifact_open", { runId: "run-original", artifactId: "artifact-original", openMode: "default" }));
  fireEvent.click(screen.getByRole("button", { name: "报告.md" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("workspace_open_item", { localRoot: workspace.localRoot, relativePath: "报告.md", openMode: "default" }));
  expect(rpc).not.toHaveBeenCalled();
  expect(props.deliverables).toEqual([deliverable]);
});

test("项目页面仍可用选定文件提交候选，不修改已有版本信息", async () => {
  const props = renderManager();
  fireEvent.click(screen.getByRole("checkbox"));
  fireEvent.change(screen.getByPlaceholderText("版本说明"), { target: { value: "下一版报告" } });
  fireEvent.click(screen.getByRole("button", { name: "提交候选" }));
  await waitFor(() => expect(rpc).toHaveBeenCalledWith("deliverable/version/create", expect.objectContaining({
    deliverableId: deliverable.deliverableId, summary: "下一版报告", localFileIds: [file.localFileId], carriers: [],
  })));
  expect(props.onChanged).toHaveBeenCalledOnce();
  expect(props.deliverables).toEqual([deliverable]);
});

test.each([{ decision: "accepted", label: "验收通过" }, { decision: "rejected", label: "退回修改" }])("项目候选仍可$label并携带正确版本和修订", async ({ decision, label }) => {
  const candidate = structuredClone(deliverable);
  candidate.status = "candidate";
  candidate.versions = [{ ...candidate.versions[0], versionId: "version-2", versionNo: 2, status: "candidate" }];
  const props = renderManager({ deliverables: [candidate] });
  fireEvent.change(screen.getByLabelText("验收人"), { target: { value: "项目负责人" } });
  fireEvent.change(screen.getByLabelText("验收结论"), { target: { value: "已核对本版文件" } });
  fireEvent.click(screen.getByRole("button", { name: label }));
  await waitFor(() => expect(rpc).toHaveBeenCalledWith("deliverable/version/review", expect.objectContaining({
    deliverableId: deliverable.deliverableId, versionId: "version-2", decision,
    expectedDeliverableRevision: 7, reviewerLabel: "项目负责人", comment: "已核对本版文件",
  })));
  expect(props.onChanged).toHaveBeenCalledOnce();
  expect(screen.queryByRole("button", { name: "提交候选" })).toBeNull();
});
