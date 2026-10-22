import { forwardRef, type ComponentProps, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

export function ConversationMarkdown(props: ComponentProps<typeof ReactMarkdown>) {
  return <ReactMarkdown {...props} remarkPlugins={[remarkGfm, ...(props.remarkPlugins ?? [])]} skipHtml />;
}

export const ConversationInput = forwardRef<HTMLTextAreaElement, ComponentProps<"textarea">>(function ConversationInput(props, ref) {
  return <textarea {...props} ref={ref} />;
});

export function conversationRunLabel(status: string): string {
  const labels: Record<string, string> = { queued: "排队中", running: "运行中", paused: "已暂停", waiting_permission: "等待权限", waiting_confirmation: "等待确认", submitting: "提交中", terminating: "正在终止", completed: "已完成", failed: "失败", interrupted: "已中断", cancelled: "已取消", permission_approved: "权限已批准", terminated: "已终止", recovery_required: "结果待核实" };
  return labels[status] ?? "状态无法确认";
}

export function conversationModelLabel(model: { modelId: string; label: string } | undefined, selected: string, loading: boolean) {
  return model?.label ?? (selected ? "原模型已不可用" : loading ? "加载模型" : "选择模型");
}

export function ConversationFields(props: { className?: string; fields: readonly { key: string; label: string; value: ReactNode; importance?: string }[] }) {
  if (!props.fields.length) return null;
  return <dl className={props.className}>{props.fields.map(field => <div key={field.key} data-importance={field.importance ?? "detail"}><dt>{field.label}</dt><dd>{field.value}</dd></div>)}</dl>;
}
