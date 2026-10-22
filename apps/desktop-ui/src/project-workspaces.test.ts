import { expect, test } from "vitest";
import { indexWorkspaceProjects, unambiguousWorkspaceProjects } from "./project-workspaces";

test("共享工作空间保留所有关联项目，不任意推断当前项目", () => {
  const projects = [
    { projectId: "a", displayName: "项目甲", workspaceIds: ["shared", "a-only"] },
    { projectId: "b", displayName: "项目乙", workspaceIds: ["shared", "b-only", "shared"] },
  ];
  const index = indexWorkspaceProjects(projects);
  expect(index.get("shared")?.map((project) => project.projectId)).toEqual(["a", "b"]);
  expect(unambiguousWorkspaceProjects(index).has("shared")).toBe(false);
  expect(unambiguousWorkspaceProjects(index).get("a-only")?.projectId).toBe("a");
  expect(unambiguousWorkspaceProjects(indexWorkspaceProjects(projects.slice(1))).get("shared")?.projectId).toBe("b");
});
