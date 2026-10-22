export interface WorkspaceProjectIdentity { projectId: string; displayName: string }

export function indexWorkspaceProjects(projects: (WorkspaceProjectIdentity & { workspaceIds: string[] })[]): Map<string, WorkspaceProjectIdentity[]> {
  const index = new Map<string, WorkspaceProjectIdentity[]>();
  for (const project of projects) {
    for (const workspaceId of project.workspaceIds) {
      const linked = index.get(workspaceId) ?? [];
      if (!linked.some((item) => item.projectId === project.projectId)) linked.push({ projectId: project.projectId, displayName: project.displayName });
      index.set(workspaceId, linked);
    }
  }
  return index;
}

export function unambiguousWorkspaceProjects(index: Map<string, WorkspaceProjectIdentity[]>): Map<string, WorkspaceProjectIdentity> {
  return new Map([...index].filter(([, projects]) => projects.length === 1).map(([workspaceId, projects]) => [workspaceId, projects[0]]));
}
