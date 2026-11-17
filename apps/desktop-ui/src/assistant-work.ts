import { invoke } from "./host";

export interface AssistantWorkItem {
  id: string;
  runId: string;
  title: string;
  projectId: string | null;
  projectName: string | null;
  taskId: string | null;
  status: string;
  waiting: { question: string; impact: string } | null;
  updatedAt: number;
}

export type WorkSupplement = Pick<AssistantWorkItem, "id" | "runId" | "title">;

export async function loadAssistantWork(runtimeSessionId: string): Promise<AssistantWorkItem[]> {
  const view = await invoke<{ runtimeSessionId: string; work: AssistantWorkItem[] }>("ai_personal_assistant_view", {
    runtimeSessionId, workOnly: true,
  });
  if (view.runtimeSessionId !== runtimeSessionId) throw new Error("ai_read_scope_changed");
  return view.work;
}
