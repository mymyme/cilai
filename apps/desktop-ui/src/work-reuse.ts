export interface WorkReuseIntent {
  token: string;
  title: string;
  conversationId: string;
  projectId: string | null;
  agentId: string;
  modelId: string | null;
  input: string;
  output: string;
}

export function reuseInstructions(intent: WorkReuseIntent): string {
  return `任务要求：\n${intent.input}\n\n执行要求：\n每次根据当前材料重新核实，沿用适用的方法和验收标准；缺少材料或权限时停止并说明原因。\n\n已完成工作的参考结果：\n${intent.output}`;
}
