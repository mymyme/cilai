import { rpc } from "./app-server-rpc";
import type { OpenPreviewParams, ReadBinaryPreviewChunkResult, ReadBinaryPreviewResult, ReleaseTextResourceResult } from "./generated/protocol";

const MAX_BYTES = 32 * 1024 * 1024;
const CHUNK_BYTES = 128 * 1024;

export async function readBinaryPreview(params: OpenPreviewParams, signal: AbortSignal) {
  const descriptor = await rpc<ReadBinaryPreviewResult>("preview/readBinary", {
    workspaceId: params.workspaceId, localFileId: params.localFileId,
  });
  if (!descriptor.ticketId) throw new Error("preview_descriptor_invalid");
  try {
    signal.throwIfAborted();
    if (!Number.isSafeInteger(descriptor.sizeBytes) || descriptor.sizeBytes <= 0 || descriptor.sizeBytes > MAX_BYTES
      || !/^[a-f0-9]{64}$/.test(descriptor.contentRevision)) throw new Error("preview_descriptor_invalid");
    const bytes = new Uint8Array(descriptor.sizeBytes);
    for (let offset = 0; offset < bytes.byteLength; offset += CHUNK_BYTES) {
      signal.throwIfAborted();
      const chunk = await rpc<ReadBinaryPreviewChunkResult>("preview/readBinaryChunk", { ticketId: descriptor.ticketId, offset });
      signal.throwIfAborted();
      const size = Math.min(CHUNK_BYTES, bytes.byteLength - offset);
      if (chunk.offset !== offset || typeof chunk.contentBase64 !== "string"
        || chunk.contentBase64.length !== Math.ceil(size / 3) * 4) throw new Error("preview_source_changed");
      const decoded = atob(chunk.contentBase64);
      if (decoded.length !== size) throw new Error("preview_source_changed");
      for (let i = 0; i < size; i++) bytes[offset + i] = decoded.charCodeAt(i);
    }
    const digest = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), n => n.toString(16).padStart(2, "0")).join("");
    signal.throwIfAborted();
    if (digest !== descriptor.contentRevision) throw new Error("preview_source_changed");
    return { bytes: bytes.buffer, contentType: descriptor.contentType, contentRevision: descriptor.contentRevision };
  } finally {
    void rpc<ReleaseTextResourceResult>("preview/releaseBinary", { ticketId: descriptor.ticketId })
      .catch(error => console.error("预览资源未能释放", error));
  }
}
