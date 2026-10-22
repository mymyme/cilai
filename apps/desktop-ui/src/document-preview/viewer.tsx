import { Select } from "../select";
import { useEffect, useMemo, useRef, useState } from "react";
import type { DocumentPreviewModel, OfficePreviewFormat, PreviewParagraph, PreviewSheet, PreviewTopic, SlideObject } from "./model";
import "./viewer.css";

export function OfficeDocumentPreview({ bytes, format }: { bytes: ArrayBuffer; format: OfficePreviewFormat }) {
  const [model, setModel] = useState<DocumentPreviewModel | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setModel(null); setError(null);
    const worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
    const timeout = window.setTimeout(() => { worker.terminate(); setError("文档解析超时。"); }, 20_000);
    worker.onmessage = (event: MessageEvent<{ model?: DocumentPreviewModel; error?: string }>) => {
      window.clearTimeout(timeout); worker.terminate();
      if (event.data.error || !event.data.model) setError(packageError(event.data.error));
      else setModel(event.data.model);
    };
    worker.onerror = () => { window.clearTimeout(timeout); worker.terminate(); setError("文档解析失败。"); };
    const copy = bytes.slice(0); worker.postMessage({ bytes: copy, format }, [copy]);
    return () => { window.clearTimeout(timeout); worker.terminate(); };
  }, [bytes, format]);
  if (error) return <div className="preview-message" role="alert"><h3>无法预览</h3><p>{error}</p></div>;
  return model ? <StructuredDocumentPreview model={model}/> : <div className="preview-message" role="status">正在解析文档</div>;
}
function packageError(reason?: string): string {
  if (reason === "preview_package_limits" || reason === "preview_file_too_large") return "文档超过预览处理上限。";
  if (reason === "preview_package_unsafe") return "文档包含不允许解析的内容。";
  if (reason === "preview_xmind_format_unsupported") return "当前 XMind 文件版本尚不支持。";
  return "文档损坏、已加密或格式不受支持。";
}
export function StructuredDocumentPreview({ model }: { model: DocumentPreviewModel }) {
  if (model.format === "docx") return <DocxPreview bytes={model.bytes}/>;
  if (model.format === "pptx") return <SlidesPreview model={model}/>;
  if (model.format === "xlsx") return <WorkbookPreview model={model}/>;
  return <MindMapPreview model={model}/>;
}
function DocxPreview({ bytes }: { bytes: ArrayBuffer }) {
  const [html, setHtml] = useState<string | null>(null); const [error, setError] = useState(false);
  useEffect(() => {
    let disposed = false; setHtml(null); setError(false);
    const body = document.createElement("div"); const styles = document.createElement("div");
    void import("docx-preview").then(({ renderAsync }) => renderAsync(bytes, body, styles, {
      useBase64URL: true, renderAltChunks: false, renderComments: false, renderChanges: false,
      ignoreLastRenderedPageBreak: false, experimental: false,
    })).then(() => {
      if (disposed) return;
      for (const element of body.querySelectorAll("a,form,script,iframe,object,embed,input,button")) {
        if (element.tagName === "A") element.replaceWith(...element.childNodes);
        else element.remove();
      }
      setHtml(`<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data: blob:; font-src data: blob:; style-src 'unsafe-inline';"><style>body{margin:0;background:#f4f4f3} .docx-wrapper{padding:20px!important} section.docx{max-width:100%;box-sizing:border-box}</style>${styles.innerHTML}</head><body>${body.innerHTML}</body></html>`);
    }).catch(() => { if (!disposed) setError(true); });
    return () => { disposed = true; body.replaceChildren(); styles.replaceChildren(); };
  }, [bytes]);
  if (error) return <div className="preview-message" role="alert">Word 文档无法解析。</div>;
  return html ? <iframe className="office-docx-frame" sandbox="" srcDoc={html} title="Word 文档预览"/> : <div className="preview-message" role="status">正在排版</div>;
}
function SlidesPreview({ model }: { model: Extract<DocumentPreviewModel, { format: "pptx" }> }) {
  const [index, setIndex] = useState(0); const [zoom, setZoom] = useState(1);
  useEffect(() => { setIndex(0); setZoom(1); }, [model]);
  const slide = model.slides[index];
  if (!slide) return <div className="preview-message">没有可显示的幻灯片。</div>;
  return <div className="office-slides">
    <div className="office-toolbar" aria-label="幻灯片导航">
      <button disabled={index === 0} onClick={() => setIndex(index - 1)} type="button">上一页</button>
      <span>第 {index + 1} / {model.slides.length} 页</span>
      <button disabled={index === model.slides.length - 1} onClick={() => setIndex(index + 1)} type="button">下一页</button>
      <button aria-label="缩小" disabled={zoom <= .5} onClick={() => setZoom(zoom - .25)} type="button">−</button><span>{Math.round(zoom * 100)}%</span><button aria-label="放大" disabled={zoom >= 2} onClick={() => setZoom(zoom + .25)} type="button">+</button>
    </div>
    <div className="office-slide-scroll"><svg className="office-slide" aria-label={`幻灯片第 ${index + 1} 页`} role="img" viewBox={`0 0 ${model.width / 12700} ${model.height / 12700}`} style={{ width: `${zoom * 100}%`, background: slide.background ?? "white", aspectRatio: `${model.width}/${model.height}` }}>
      {slide.objects.map((object) => <SlideElement key={object.id} object={object}/>)}
    </svg></div>
  </div>;
}
function SlideElement({ object }: { object: SlideObject }) {
  const scale = 12700; const { x, y, width, height, rotation = 0 } = object.geometry;
  const w = width / scale; const h = height / scale;
  const frame = { x: 0, y: 0, width: w, height: h };
  const style = { fill: object.fill ?? "none", stroke: object.stroke ?? "none", strokeWidth: 1 };
  let shape;
  if (object.kind === "placeholder") shape = <><rect {...frame} fill="#f5f5f5" stroke="#b8bdc4" strokeDasharray="4 3"/><foreignObject {...frame}><div className="office-slide-placeholder">{object.placeholder}</div></foreignObject></>;
  else if (object.kind === "image") {
    const crop = object.crop ?? { left: 0, top: 0, right: 0, bottom: 0 };
    const visibleX = Math.max(.01, 1 - crop.left - crop.right); const visibleY = Math.max(.01, 1 - crop.top - crop.bottom);
    shape = <svg {...frame} overflow="hidden" viewBox={`0 0 ${w} ${h}`}><image href={object.image} x={-w * crop.left / visibleX} y={-h * crop.top / visibleY} width={w / visibleX} height={h / visibleY} preserveAspectRatio="none"/></svg>;
  } else if (object.kind === "table") shape = <foreignObject {...frame}><table className="office-slide-table"><tbody>{object.rows?.map((row, i) => <tr key={i}>{row.map((cell, j) => <td key={j}>{object.cellParagraphs?.[i]?.[j]?.length ? <TextParagraphs paragraphs={object.cellParagraphs[i][j]}/> : cell}</td>)}</tr>)}</tbody></table></foreignObject>;
  else {
    const type = object.shape ?? "rect";
    const geometry = type === "ellipse" ? <ellipse cx={w / 2} cy={h / 2} rx={w / 2} ry={h / 2} {...style}/> : type === "triangle" ? <polygon points={`${w / 2},0 ${w},${h} 0,${h}`} {...style}/> : type === "diamond" ? <polygon points={`${w / 2},0 ${w},${h / 2} ${w / 2},${h} 0,${h / 2}`} {...style}/> : type === "line" ? <line x1="0" y1="0" x2={w} y2={h} stroke={object.stroke ?? "#333"}/> : ["rect", "roundRect"].includes(type) ? <rect {...frame} rx={type === "roundRect" ? Math.min(w, h) * .15 : 0} {...style}/> : <rect {...frame} fill="none" stroke="#aaa" strokeDasharray="3 3"/>;
    shape = <>{geometry}<foreignObject {...frame}><div className="office-slide-text"><TextParagraphs paragraphs={object.paragraphs ?? []}/></div></foreignObject></>;
  }
  return <g transform={`translate(${x / scale} ${y / scale}) rotate(${rotation} ${w / 2} ${h / 2})`}>{shape}</g>;
}
function TextParagraphs({ paragraphs }: { paragraphs: PreviewParagraph[] }) {
  return <>{paragraphs.map((p, index) => <p key={index} style={{ textAlign: p.align }}>{p.runs.map((run, i) => <span key={i} style={{ fontSize: `${run.size ?? 18}px`, color: run.color, fontFamily: run.fontFamily, fontWeight: run.bold ? 700 : undefined, fontStyle: run.italic ? "italic" : undefined, textDecoration: run.underline ? "underline" : undefined }}>{run.text}</span>)}</p>)}</>;
}
function WorkbookPreview({ model }: { model: Extract<DocumentPreviewModel, { format: "xlsx" }> }) {
  const [index, setIndex] = useState(0);
  useEffect(() => setIndex(0), [model]);
  return <div className="office-workbook"><div className="office-toolbar"><label>工作表 <Select ariaLabel="工作表" value={String(index)} onChange={(value) => setIndex(Number(value))} options={model.sheets.map((sheet, i) => ({value:String(i),label:sheet.name}))}/></label><span>公式结果：{model.calculationState === "stale" ? "待重算" : "文件缓存"}</span></div>{model.sheets[index] ? <SheetGrid key={model.sheets[index].id} sheet={model.sheets[index]}/> : <div className="preview-message">没有工作表。</div>}</div>;
}
export function columnLabel(index: number): string { let result = ""; let value = index; while (value > 0) { value -= 1; result = String.fromCharCode(65 + value % 26) + result; value = Math.floor(value / 26); } return result; }
function SheetGrid({ sheet }: { sheet: PreviewSheet }) {
  const [position, setPosition] = useState({ top: 0, left: 0 }); const [selected, setSelected] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const cells = useMemo(() => new Map(sheet.cells.map((cell) => [`${cell.row}:${cell.column}`, cell])), [sheet]);
  const firstRow = Math.max(1, Math.floor(position.top / 28)); const firstColumn = Math.max(1, Math.floor(position.left / 120));
  const lastRow = Math.min(sheet.rowCount, firstRow + 45); const lastColumn = Math.min(sheet.columnCount, firstColumn + 18);
  const displayed = [];
  for (let row = firstRow; row <= lastRow; row += 1) for (let column = firstColumn; column <= lastColumn; column += 1) {
    const merge = sheet.merges.find((m) => row >= m.startRow && row <= m.endRow && column >= m.startColumn && column <= m.endColumn);
    if (merge && (row !== Math.max(firstRow, merge.startRow) || column !== Math.max(firstColumn, merge.startColumn))) continue;
    const cell = cells.get(`${merge?.startRow ?? row}:${merge?.startColumn ?? column}`);
    const address = cell?.address ?? `${columnLabel(column)}${row}`;
    displayed.push(<div aria-colindex={column} aria-rowindex={row} className="office-cell" key={`${row}:${column}`} onClick={() => setSelected(address)} role="gridcell" style={{ top: row * 28, left: column * 120 - 72, width: 120 * (merge ? Math.min(lastColumn, merge.endColumn) - column + 1 : 1), height: 28 * (merge ? Math.min(lastRow, merge.endRow) - row + 1 : 1) }} title={cell?.formula ? `${address}: =${cell.formula}\n缓存值: ${cell.value}` : `${address}: ${cell?.value ?? ""}`}>{cell?.value}</div>);
  }
  const selectedCell = sheet.cells.find((cell) => cell.address === selected);
  return <><div className="office-formula" aria-label="单元格内容"><span>{selected ?? ""}</span><span>{selectedCell?.formula ? `=${selectedCell.formula}` : selectedCell?.value ?? ""}</span></div><div aria-colcount={sheet.columnCount} aria-label={sheet.name} aria-rowcount={sheet.rowCount} className="office-grid" onScroll={(event) => setPosition({ top: event.currentTarget.scrollTop, left: event.currentTarget.scrollLeft })} ref={ref} role="grid"><div style={{ width: sheet.columnCount * 120 + 48, height: sheet.rowCount * 28 + 28, position: "relative" }}>{displayed}{Array.from({ length: lastRow - firstRow + 1 }, (_, i) => firstRow + i).map((row) => <div className="office-row-header" key={`r${row}`} style={{ top: row * 28, left: position.left }}>{row}</div>)}{Array.from({ length: lastColumn - firstColumn + 1 }, (_, i) => firstColumn + i).map((column) => <div className="office-column-header" key={`c${column}`} style={{ left: column * 120 - 72, top: position.top }}>{columnLabel(column)}</div>)}</div></div></>;
}
function MindMapPreview({ model }: { model: Extract<DocumentPreviewModel, { format: "xmind" }> }) {
  const [index, setIndex] = useState(0); useEffect(() => setIndex(0), [model]);
  const sheet = model.sheets[index];
  return <div className="office-mindmap"><div className="office-toolbar"><label>画布 <Select ariaLabel="思维导图画布" value={String(index)} onChange={(value) => setIndex(Number(value))} options={model.sheets.map((s, i) => ({value:String(i),label:s.title || `画布 ${i + 1}`}))}/></label></div><div className="office-topic-scroll" role="tree" aria-label="思维导图">{sheet ? <Topic topic={sheet.root}/> : <div className="preview-message">没有主题。</div>}</div></div>;
}
function Topic({ topic }: { topic: PreviewTopic }) { return <details className="office-topic" open role="treeitem"><summary>{topic.title || "未命名主题"}</summary>{topic.notes ? <p className="office-topic-notes">{topic.notes}</p> : null}{topic.children.length ? <div role="group">{topic.children.map((item) => <Topic key={item.id} topic={item}/>)}</div> : null}</details>; }
