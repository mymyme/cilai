import { invoke } from "./host";

export interface ConversationSessionSnapshot {
  sessionId: string;
  revision: number;
  values: Record<string, unknown>;
}
export interface ConversationSessionChange { key: string; previous: unknown; value: unknown }
export const loadConversationSession = () => invoke<ConversationSessionSnapshot>("ai_conversation_session_get");
export const patchConversationSession = (sessionId: string, changes: ConversationSessionChange[]) =>
  invoke<ConversationSessionSnapshot>("ai_conversation_session_patch", { request: { sessionId, changes } });
