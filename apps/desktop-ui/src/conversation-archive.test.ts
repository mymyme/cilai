import { afterEach, expect, test, vi } from "vitest";
import { emptyAiSnapshot, type AiConversation } from "./ai-runtime";
import { conversationArchiveChanged, invalidateArchiveOperations, mergeArchivedConversations, setConversationsArchived } from "./conversation-archive";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
afterEach(() => { invoke.mockReset(); });
test("旧归档回执不能覆盖更高修订的恢复或删除结果", () => {
  const current = { ...emptyAiSnapshot, stateRevision: 9 };
  const archived = { conversationId: "a", archived: true } as AiConversation;
  expect(mergeArchivedConversations(current, { stateRevision: 8, conversations: [archived], changedConversationIds: ["a"], failures: [] })).toBe(current);
});

test("切换企业后旧归档回执不发布到新企业页面", async () => {
  let complete!: (value: unknown) => void;
  invoke.mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
  const changed = vi.fn();
  window.addEventListener(conversationArchiveChanged, changed);
  try {
    const operation = setConversationsArchived(["a"], true);
    const rejected = expect(operation).rejects.toThrow("企业已切换");
    invalidateArchiveOperations();
    complete({ stateRevision: 2, conversations: [{ conversationId: "a", archived: true }], changedConversationIds: ["a"], failures: [] });
    await rejected;
    expect(changed).not.toHaveBeenCalled();
  } finally { window.removeEventListener(conversationArchiveChanged, changed); }
});
