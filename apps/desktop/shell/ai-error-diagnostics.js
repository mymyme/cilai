// Only these public codes and validated runtime/request IDs can leave a failure. Never persist
// the error message, output, tool arguments, paths, prompts or conversation IDs.
const publicAiCodes = new Set([
  "AI-REQ-001", "AI-REQ-002", "AI-REQ-003",
  "AI-AUTH-001", "AI-AUTH-002", "AI-AUTH-003",
  "AI-QUOTA-001", "AI-QUOTA-002", "AI-LIMIT-001", "AI-LIMIT-002",
  "AI-EXEC-001", "AI-EXEC-002", "AI-EXEC-003", "AI-MODEL-001",
  "AI-SVC-001", "AI-SVC-002", "AI-SYS-001",
]);
const uuidPattern = "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}";
const uuid = new RegExp(`^${uuidPattern}$`, "i");
function safeId(value) { return typeof value === "string" && uuid.test(value) ? value.toLowerCase() : null; }
const runId = new RegExp(`^(?:run_|app_run_)?${uuidPattern}$`, "i");
// Runtime emits run_<UUID> and app_run_<UUID>; gateway request IDs stay UUID-only.
function safeRunId(value) { return typeof value === "string" && runId.test(value) ? value.toLowerCase() : null; }
function publicCodes(value) {
  return typeof value === "string" ? [...new Set(value.match(/\bAI-[A-Z]+-\d{3}\b/g) ?? [])].filter(code => publicAiCodes.has(code)) : [];
}
function requestId(value) {
  if (typeof value !== "string") return null;
  const match = value.match(new RegExp(`(?:请求编号\\s*[:：]\\s*|["']request_id["']\\s*:\\s*["'])(${uuidPattern})(?=$|[\\s"',}])`, "i"));
  return safeId(match?.[1]);
}
function failureTime(value) {
  // AiAgentRun.completedAt is Unix epoch milliseconds, not observation time.
  return Number.isSafeInteger(value) && value >= 946684800000 && value <= Date.now()
    ? new Date(value).toISOString() : null;
}
module.exports = { publicAiCodes, publicCodes, safeId, safeRunId, requestId, failureTime };
