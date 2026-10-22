import { invoke } from "./host";
import { rpc } from "./app-server-rpc";
import type { UnifiedSearchGroup, UnifiedSearchParams, UnifiedSearchResult } from "./generated/protocol";

export async function searchSources(params: UnifiedSearchParams): Promise<UnifiedSearchResult> {
  if (params.scope.some(source => source !== "local" && source !== "conversations")) throw new Error("personal_search_source_removed");
  const native = params.scope.includes("conversations");
  const scope = params.scope.filter((source) => source !== "conversations");
  const result: UnifiedSearchResult = { requestId: "native-search", authorizationRevision: null, groups: [], partialFailures: [] };
  const requests: Array<Promise<void>> = [];
  const businessCursors = params.cursors.filter((cursor) => !cursor.providerId.startsWith("native-"));
  if (scope.length && (!params.cursors.length || businessCursors.length)) requests.push(rpc<UnifiedSearchResult>("unifiedSearch/query", { ...params, scope, cursors: businessCursors }).then((value) => {
    result.requestId = value.requestId; result.authorizationRevision = value.authorizationRevision;
    result.groups.push(...value.groups); result.partialFailures.push(...value.partialFailures);
  }));
  if (native) for (const archived of [false, true]) {
    const providerId = archived ? "native-archived-conversations" : "native-conversations";
    if (params.cursors.length && !params.cursors.some((cursor) => cursor.providerId === providerId)) continue;
    requests.push(invoke<UnifiedSearchGroup>("ai_conversation_search", {
      query: params.query, archived, limit: params.limit,
      cursor: params.cursors.find((cursor) => cursor.providerId === providerId)?.value ?? null,
    }).then((group) => { result.groups.push(group); }).catch((error) => {
      result.partialFailures.push({ groupId: providerId, providerId, code: "conversation_search_failed", message: String(error) });
    }));
  }
  await Promise.all(requests);
  result.groups.sort((a, b) => {
    const order = ["native-conversations", "native-archived-conversations"];
    return (order.indexOf(a.providerId) + 1 || 3) - (order.indexOf(b.providerId) + 1 || 3);
  });
  return result;
}
