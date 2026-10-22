import { invoke } from "./host";
import type { AiConversation, AiRuntimeSnapshot } from "./ai-runtime";

export interface ConversationArchiveResult {
  stateRevision: number;
  conversations: AiConversation[];
  changedConversationIds: string[];
  failures: { conversationId: string; reason: string }[];
}
export interface ConversationArchiveChange extends ConversationArchiveResult { archived: boolean }
export const conversationArchiveChanged = "cilai:conversation-archive-changed";

let profileGeneration = 0;
export function invalidateArchiveOperations() { profileGeneration += 1; }

export async function setConversationsArchived(conversationIds: string[], archived: boolean): Promise<ConversationArchiveResult> {
  const generation = profileGeneration;
  const result = await invoke<ConversationArchiveResult>("ai_conversation_set_archived", { request: { conversationIds, archived } });
  if (generation !== profileGeneration) throw new Error("企业已切换，原归档操作的回执已失效。");
  if (result.conversations.length) window.dispatchEvent(new CustomEvent<ConversationArchiveChange>(conversationArchiveChanged, { detail: { ...result, archived } }));
  return result;
}

export function mergeArchivedConversations(snapshot: AiRuntimeSnapshot, result: ConversationArchiveResult): AiRuntimeSnapshot {
  if (result.stateRevision < snapshot.stateRevision) return snapshot;
  const updates = new Map(result.conversations.map((item) => [item.conversationId, item]));
  const conversations = snapshot.conversations.map((item) => {
    const updated = updates.get(item.conversationId);
    updates.delete(item.conversationId);
    return updated ?? item;
  });
  return { ...snapshot, stateRevision: result.stateRevision, conversations: [...conversations, ...updates.values()] };
}
