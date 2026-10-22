import { readDocumentPackage } from "./package";
import type { OfficePreviewFormat } from "./model";
self.onmessage = (event: MessageEvent<{ bytes: ArrayBuffer; format: OfficePreviewFormat }>) => {
  try { self.postMessage({ model: readDocumentPackage(event.data.bytes, event.data.format) }); }
  catch (error) { self.postMessage({ error: error instanceof Error ? error.message : "preview_package_invalid" }); }
};
