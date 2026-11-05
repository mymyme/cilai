import { invoke } from "./host";

export async function readAiMessageReceipt(profileId: string | null | undefined, clientMessageId: string, conversationId: string | null) {
  if (!profileId) throw new Error("ai_message_receipt_scope_mismatch");
  const result = await invoke<{ status: "recorded" | "preparing" | "failed" | "absent"; conversationId: string | null }>(
    "ai_message_receipt", { profileId, clientMessageId, conversationId },
  );
  if (!result || !["recorded", "preparing", "failed", "absent"].includes(result.status)
    || (result.status === "absent" ? result.conversationId !== null
      : typeof result.conversationId !== "string" || (conversationId !== null && result.conversationId !== conversationId))) {
    throw new Error("ai_message_receipt_unconfirmed");
  }
  return result;
}
