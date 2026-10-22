const OPAQUE_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/**
 * 模型已从企业目录里下线时，界面上只剩下它的编号。像 "agnes-3.0-flash" 这种可读编号可以直接给人看；
 * 网关生成的 UUID 对使用者没有任何意义，统一称作"原模型"。
 */
export function displayModelId(modelId: string): string {
  return OPAQUE_ID.test(modelId.trim()) ? "原模型" : modelId;
}
