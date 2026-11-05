import { parseMarkdownDocument, readMarkdownPage, type MarkdownDocument } from "./file-markdown-model";
import type { FileMarkdownRequest, FileMarkdownResponse } from "./file-markdown-protocol";

let document: MarkdownDocument | null = null;
self.onmessage = (event: MessageEvent<FileMarkdownRequest>) => {
  const request = event.data;
  let response: FileMarkdownResponse;
  try {
    if (request.type === "load") {
      document = null;
      document = parseMarkdownDocument(request.content);
    }
    if (!document) throw new Error("markdown_not_loaded");
    const page = readMarkdownPage(document, request.type === "page" ? request.index : 0, request.type === "page" && request.expand);
    response = { type: "page", requestId: request.requestId, page };
  } catch {
    response = { type: "error", requestId: request.requestId };
  }
  self.postMessage(response);
};
