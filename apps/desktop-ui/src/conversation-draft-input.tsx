import { ConversationInput } from "@cilai/app-ui/conversation";
import { forwardRef, useCallback, useLayoutEffect, useRef, type KeyboardEvent, type TextareaHTMLAttributes } from "react";

export function isConversationSendKey(event: KeyboardEvent<HTMLElement>) {
  return event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing && event.nativeEvent.keyCode !== 229;
}

type Props = Omit<TextareaHTMLAttributes<HTMLTextAreaElement>, "value" | "defaultValue" | "onChange"> & {
  value: string;
  onValueChange: (value: string) => void;
};

export const ConversationDraftInput = forwardRef<HTMLTextAreaElement, Props>(function ConversationDraftInput(
  { value, onValueChange, ...props }, forwardedRef,
) {
  const input = useRef<HTMLTextAreaElement | null>(null);
  const initialValue = useRef(value);
  const bindInput = useCallback((node: HTMLTextAreaElement | null) => {
    input.current = node;
    if (typeof forwardedRef === "function") forwardedRef(node);
    else if (forwardedRef) forwardedRef.current = node;
  }, [forwardedRef]);
  useLayoutEffect(() => {
    // 原生字符可能已进入编辑器而 input 事件仍在排队。无关重渲染不得恢复旧值；
    // 仅明确变化的外部草稿（切换会话、发送后清空、恢复输入）可以改写编辑器。
    if (input.current && input.current.value !== value) input.current.value = value;
  }, [value]);
  return <ConversationInput {...props} defaultValue={initialValue.current}
    onChange={(event) => onValueChange(event.currentTarget.value)} ref={bindInput}/>;
});
