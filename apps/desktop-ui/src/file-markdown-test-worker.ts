import { parseMarkdownDocument, readMarkdownPage, type MarkdownDocument } from "./file-markdown-model";
import type { FileMarkdownRequest, FileMarkdownResponse } from "./file-markdown-protocol";

export class FileMarkdownTestWorker {
    private document: MarkdownDocument | null = null;
    private closed = false;
    onmessage?: (event: { data: FileMarkdownResponse }) => void;
    postMessage(request: FileMarkdownRequest) {
      queueMicrotask(() => {
        if (this.closed) return;
        if (request.type === "load") this.document = parseMarkdownDocument(request.content);
        if (this.document) this.onmessage?.({ data: { type: "page", requestId: request.requestId,
          page: readMarkdownPage(this.document, request.type === "page" ? request.index : 0, request.type === "page" && request.expand) } });
      });
    }
    terminate() { this.closed = true; }
}
