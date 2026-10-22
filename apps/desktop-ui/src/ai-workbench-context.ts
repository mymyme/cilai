import type { WorkspaceFileItem, WorkspaceSummary } from "./generated/protocol";
import type { AiContext } from "./ai-runtime";

interface ProjectIdentity {
  projectId: string;
  displayName: string;
}

export function workspaceContext(
  workspace: WorkspaceSummary,
  file: WorkspaceFileItem | null,
  project: ProjectIdentity | null = null,
): AiContext {
  return {
    workspaceId: workspace.workspaceId,
    workspaceName: workspace.displayName,
    workspaceRoot: workspace.workspaceKind === "local" ? workspace.localRoot : "",
    teamSpaceId: workspace.teamSpaceId,
    projectId: project?.projectId ?? null,
    projectName: project?.displayName ?? null,
    phaseId: null,
    phaseName: null,
    taskId: null,
    taskName: null,
    deliverableId: null,
    deliverableName: null,
    localFileId: file?.localFileId ?? null,
    relativePath: file?.relativePath ?? null,
    fileRevision: file?.stateRevision ?? null,
    referencedMaterials: file
      ? [{
        localFileId: file.localFileId,
        relativePath: file.relativePath,
        fileRevision: file.stateRevision,
        workspaceId: workspace.workspaceId,
      }]
      : [],
  };
}

export function unclassifiedConversationContext(): AiContext {
  return {
    workspaceId: null,
    workspaceName: null,
    workspaceRoot: "",
    projectId: null,
    projectName: null,
    phaseId: null,
    phaseName: null,
    taskId: null,
    taskName: null,
    deliverableId: null,
    deliverableName: null,
    localFileId: null,
    relativePath: null,
    fileRevision: null,
    referencedMaterials: [],
  };
}
