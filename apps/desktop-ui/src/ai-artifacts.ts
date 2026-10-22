import type { AiAgentRun } from "./ai-runtime";

export type ConversationArtifact = AiAgentRun["artifacts"][number];
export type ConversationArtifactEntry = { runId: string; artifact: ConversationArtifact };

export function conversationFileArtifacts(runs: Array<Pick<AiAgentRun, "runId" | "artifacts"> & { superseded?: boolean }>): ConversationArtifactEntry[] {
  const seenPaths = new Set<string>();
  return runs.filter((run) => !run.superseded).reverse().flatMap((run) => [...run.artifacts].reverse().map((artifact) => ({ runId: run.runId, artifact }))).filter(({ artifact }) => {
    if (artifact.artifactType === "assistant-response" || !artifact.path?.trim()) return false;
    const path = artifact.path.replace(/\\/g, "/").replace(/\/{2,}/g, "/");
    const identity = artifact.version ? `${path}:${artifact.version.digest}` : path;
    if (seenPaths.has(identity)) return false;
    seenPaths.add(identity);
    return true;
  });
}

export function artifactFileName(path: string): string {
  return path.replaceAll("\\", "/").split("/").filter(Boolean).at(-1) ?? path;
}
