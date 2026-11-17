/**
 * 持续状态的报错被用户关闭后，在本次页面会话内记住“同一个问题”，同一状态切页、刷新不再重复弹出。
 * 只保存在内存中：没有持久化的已读记录，应用重新加载后问题仍在会再次提示；状态键变化（新问题）即是新的提醒。
 */
export function createClosedStateRegistry() {
  const keyById = new Map<string, string>();
  const closed = new Set<string>();
  return {
    /** 通知展示前登记编号对应的状态键。已被关闭的状态返回 false，调用方不应再展示。 */
    admit(id: string, stateKey?: string): boolean {
      if (!stateKey) return true;
      if (closed.has(stateKey)) return false;
      keyById.set(id, stateKey);
      return true;
    },
    /** 用户主动关闭才记住状态；程序撤下通知（问题解除、页面卸载）和定时收起都不记。 */
    dismissed(id: string, reason?: "close" | "timeout") {
      const key = keyById.get(id);
      keyById.delete(id);
      if (key && reason === "close") closed.add(key);
    },
    reset() {
      keyById.clear();
      closed.clear();
    },
  };
}
