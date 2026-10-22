import { rpc } from "./app-server-rpc";
import type { FileHydrationResult } from "./generated/protocol";

function waitForPoll(signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const onAbort = () => {
      clearTimeout(timer);
      reject(new DOMException("file_hydration_cancelled", "AbortError"));
    };
    const timer = setTimeout(() => {
      signal.removeEventListener("abort", onAbort);
      resolve();
    }, 250);
    signal.addEventListener("abort", onAbort, { once: true });
    if (signal.aborted) onAbort();
  });
}

export async function hydrateFileForRead(workspaceId: string, relativePath: string, signal: AbortSignal, onWaiting: () => void): Promise<boolean> {
  signal.throwIfAborted();
  let operationId: string | null | undefined;
  let downloaded = false;
  try {
    let result = await rpc<FileHydrationResult>("file/hydrate", { workspaceId, relativePath });
    operationId = result.operationId;
    signal.throwIfAborted();
    while (result.state === "downloading") {
      if (!operationId) throw new Error("file_hydration_operation_required");
      downloaded = true;
      onWaiting();
      await waitForPoll(signal);
      signal.throwIfAborted();
      result = await rpc<FileHydrationResult>("file/hydrate", { workspaceId, relativePath, operationId });
      signal.throwIfAborted();
    }
    if (result.state !== "ready") throw new Error("file_hydration_cancelled");
    operationId = undefined;
    return downloaded;
  } finally {
    if (signal.aborted && operationId) {
      await rpc("file/hydrate", { workspaceId, relativePath, operationId, cancel: true });
    }
  }
}
