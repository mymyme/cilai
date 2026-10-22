export type OfficePreviewFormat = "docx" | "pptx" | "xlsx" | "xmind";
export interface PreviewTextRun { text: string; bold?: boolean; italic?: boolean; underline?: boolean; size?: number; color?: string; fontFamily?: string; }
export interface PreviewParagraph { runs: PreviewTextRun[]; align?: "left" | "center" | "right"; }
export interface SlideGeometry { x: number; y: number; width: number; height: number; rotation?: number; }
export interface SlideObject {
  id: string; kind: "shape" | "text" | "image" | "table" | "placeholder";
  geometry: SlideGeometry; paragraphs?: PreviewParagraph[]; fill?: string; stroke?: string;
  shape?: string; image?: string; crop?: { left: number; top: number; right: number; bottom: number };
  rows?: string[][]; cellParagraphs?: PreviewParagraph[][][]; placeholder?: string;
}
export interface PreviewSlide { id: string; objects: SlideObject[]; background?: string; }
export interface PreviewCell { address: string; row: number; column: number; value: string; formula?: string; }
export interface PreviewSheet { id: string; name: string; cells: PreviewCell[]; rowCount: number; columnCount: number; merges: { startRow: number; startColumn: number; endRow: number; endColumn: number }[]; }
export interface PreviewTopic { id: string; title: string; notes?: string; children: PreviewTopic[]; }
export type DocumentPreviewModel =
  | { format: "docx"; bytes: ArrayBuffer }
  | { format: "pptx"; width: number; height: number; slides: PreviewSlide[] }
  | { format: "xlsx"; sheets: PreviewSheet[]; calculationState: "cached" | "stale" }
  | { format: "xmind"; sheets: { id: string; title: string; root: PreviewTopic }[] };
