import { invoke } from "./host";

import type { ConversationSessionSnapshot, ConversationSessionChange } from "./generated/protocol";
export type { ConversationSessionSnapshot, ConversationSessionChange } from "./generated/protocol";

export const loadConversationSession = () => invoke<ConversationSessionSnapshot>("ai_conversation_session_get");
export const patchConversationSession = (sessionId: string, changes: ConversationSessionChange[]) =>
  invoke<ConversationSessionSnapshot>("ai_conversation_session_patch", { request: { sessionId, changes } });
