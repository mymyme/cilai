export type OfficePreviewFormat = "docx" | "pptx" | "xlsx" | "xmind";
export interface PreviewTextRun { text: string; bold?: boolean; italic?: boolean; underline?: boolean; size?: number; color?: string; fontFamily?: string; }
export interface PreviewParagraph { runs: PreviewTextRun[]; align?: "left" | "center" | "right"; }
export interface SlideGeometry { x: number; y: number; width: number; height: number; rotation?: number; }
export interface SlideObject {
  id: string; kind: "shape" | "text" | "image" | "table" | "chart" | "group" | "error" | "placeholder";
  geometry: SlideGeometry; paragraphs?: PreviewParagraph[]; fill?: string; stroke?: string;
  shape?: string; image?: string; crop?: { left: number; top: number; right: number; bottom: number };
  rows?: string[][]; cellParagraphs?: PreviewParagraph[][][]; placeholder?: string;
  chart?: PreviewChart; children?: SlideObject[]; viewport?: { x: number; y: number; width: number; height: number }; error?: string;
}
export interface PreviewSlide { id: string; objects: SlideObject[]; background?: string; }
export interface PreviewCell { address: string; row: number; column: number; value: string; valueType?: "number" | "text" | "boolean" | "error"; formula?: string; cachedValue?: string; calculatedValue?: string | number | boolean | null; formulaError?: string; numberFormat?: string; }
export interface PreviewSheet { id: string; name: string; cells: PreviewCell[]; rowCount: number; columnCount: number; merges: { startRow: number; startColumn: number; endRow: number; endColumn: number }[]; }
export interface PreviewTopic { id: string; title: string; notes?: string; children: PreviewTopic[]; }
export type DocumentPreviewModel =
  | { format: "docx"; bytes: ArrayBuffer }
  | { format: "pptx"; width: number; height: number; slides: PreviewSlide[] }
  | { format: "xlsx"; sheets: PreviewSheet[]; calculationState: "recalculated" | "error"; formulaCount: number; calculationErrors: number }
  | { format: "xmind"; sheets: { id: string; title: string; root: PreviewTopic }[] };

export type PreviewChartType = "bar" | "line" | "area" | "pie" | "doughnut" | "scatter" | "radar";
export interface PreviewChartSeries { name: string; values: (number | null)[]; xValues?: (number | null)[]; color: string; }
export interface PreviewChartLayer { type: PreviewChartType; series: PreviewChartSeries[]; grouping: "standard" | "stacked" | "percentStacked"; horizontal: boolean; holeSize: number; }
export interface PreviewChart {
  blankDisplay: "gap" | "zero" | "span"; title: string; categories: string[]; layers: PreviewChartLayer[]; legend: boolean; showValues: boolean; showPercent: boolean; min?: number; max?: number; logBase?: number; reversed?: boolean; xMin?: number; xMax?: number; xLogBase?: number; xReversed?: boolean; }
