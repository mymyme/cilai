import { unzipSync, zipSync } from "fflate";
import { DOMParser, XMLSerializer, type Element, type Document } from "@xmldom/xmldom";
import type { DocumentPreviewModel, OfficePreviewFormat, PreviewParagraph, PreviewSheet, PreviewSlide, PreviewTopic, SlideGeometry, SlideObject } from "./model";

const MAX_ENTRIES = 4096;
const MAX_EXPANDED = 128 * 1024 * 1024;
const MAX_XML = 12 * 1024 * 1024;
const MAX_OBJECTS = 100_000;
const decoder = new TextDecoder("utf-8", { fatal: true });
type Package = Record<string, Uint8Array>;

export function readDocumentPackage(bytes: ArrayBuffer, format: OfficePreviewFormat): DocumentPreviewModel {
  if (bytes.byteLength > 32 * 1024 * 1024) throw new Error("preview_file_too_large");
  let entries = 0; let expanded = 0;
  const names = new Set<string>();
  const pkg = unzipSync(new Uint8Array(bytes), { filter(entry) {
    entries += 1; expanded += entry.originalSize;
    if (entries > MAX_ENTRIES || expanded > MAX_EXPANDED || entry.originalSize > 48 * 1024 * 1024 || (entry.size > 0 && entry.originalSize / entry.size > 1000)) throw new Error("preview_package_limits");
    if (!entry.name || entry.name.startsWith("/") || entry.name.includes("\\") || entry.name.split("/").some((part) => part === ".." || part === ".") || names.has(entry.name)) throw new Error("preview_package_invalid");
    names.add(entry.name);
    return !entry.name.endsWith("/");
  } });
  for (const [name, data] of Object.entries(pkg)) {
    if (name.endsWith(".xml") || name.endsWith(".rels")) {
      if (data.byteLength > MAX_XML) throw new Error("preview_package_limits");
      if (/<!\s*(?:DOCTYPE|ENTITY)\b/i.test(decoder.decode(data))) throw new Error("preview_package_unsafe");
    }
  }
  if (format === "docx") {
    xml(pkg, "word/document.xml");
    for (const name of Object.keys(pkg).filter((name) => name.endsWith(".rels"))) {
      const doc = xml(pkg, name);
      for (const rel of elements(doc, "Relationship")) if (rel.getAttribute("TargetMode") === "External" || /^[a-z]+:/i.test(rel.getAttribute("Target") ?? "")) rel.parentNode?.removeChild(rel);
      pkg[name] = new TextEncoder().encode(new XMLSerializer().serializeToString(doc));
    }
    return { format, bytes: new Uint8Array(zipSync(pkg, { level: 0 })).buffer };
  }
  if (format === "pptx") return pptx(pkg);
  if (format === "xlsx") return xlsx(pkg);
  return xmind(pkg);
}

function xml(pkg: Package, name: string): Document {
  const value = pkg[name];
  if (!value) throw new Error("preview_package_invalid");
  const doc = new DOMParser({ onError: (level) => { if (level !== "warning") throw new Error("preview_package_invalid"); } }).parseFromString(decoder.decode(value), "text/xml");
  if (!doc.documentElement || doc.getElementsByTagName("parsererror").length > 0) throw new Error("preview_package_invalid");
  return doc;
}
function elements(parent: Document | Element, name: string): Element[] {
  return Array.from(parent.getElementsByTagName("*")).filter((element) => element.localName === name);
}
function child(parent: Element | Document | undefined, name: string): Element | undefined {
  if (!parent) return undefined;
  return Array.from(parent.childNodes).find((node) => node.nodeType === 1 && (node as Element).localName === name) as Element | undefined;
}
function children(parent: Element | Document | undefined, name?: string): Element[] {
  if (!parent) return [];
  return Array.from(parent.childNodes).filter((node) => node.nodeType === 1 && (!name || (node as Element).localName === name)) as Element[];
}
function num(element: Element | undefined, name: string, defaultValue = 0): number {
  const value = Number(element?.getAttribute(name) ?? defaultValue);
  if (!Number.isFinite(value)) throw new Error("preview_package_invalid");
  return value;
}
function resolvePart(base: string, target: string): string {
  if (!target || target.includes("\\") || /^[a-z]+:/i.test(target)) throw new Error("preview_package_unsafe");
  const parts = target.startsWith("/") ? [] : base.split("/").slice(0, -1);
  for (const part of target.split("/")) {
    if (part === "..") { if (!parts.length) throw new Error("preview_package_unsafe"); parts.pop(); }
    else if (part && part !== ".") parts.push(part);
  }
  return parts.join("/");
}
function relationships(pkg: Package, part: string): Map<string, { target: string; type: string }> {
  const split = part.lastIndexOf("/");
  const relPath = `${part.slice(0, split + 1)}_rels/${part.slice(split + 1)}.rels`;
  if (!pkg[relPath]) return new Map();
  return new Map(elements(xml(pkg, relPath), "Relationship").filter((rel) => rel.getAttribute("TargetMode") !== "External").map((rel) => [rel.getAttribute("Id") ?? "", { target: resolvePart(part, rel.getAttribute("Target") ?? ""), type: rel.getAttribute("Type") ?? "" }]));
}
function text(parent: Element): string { return elements(parent, "t").map((item) => item.textContent ?? "").join(""); }
function dataImage(pkg: Package, target: string | undefined): string | undefined {
  if (!target || !pkg[target]) return undefined;
  const extension = target.split(".").pop()?.toLowerCase();
  const mime = extension === "png" ? "image/png" : extension === "jpg" || extension === "jpeg" ? "image/jpeg" : extension === "gif" ? "image/gif" : extension === "webp" ? "image/webp" : null;
  if (!mime) return undefined;
  const bytes = pkg[target];
  let binary = "";
  for (let i = 0; i < bytes.length; i += 8192) binary += String.fromCharCode(...bytes.subarray(i, i + 8192));
  return `data:${mime};base64,${btoa(binary)}`;
}
function color(parent: Element | undefined, theme: Map<string, string>): string | undefined {
  if (!parent) return undefined;
  const value = elements(parent, "srgbClr")[0]?.getAttribute("val") ?? theme.get(elements(parent, "schemeClr")[0]?.getAttribute("val") ?? "") ?? elements(parent, "sysClr")[0]?.getAttribute("lastClr");
  return value && /^[\da-f]{6}$/i.test(value) ? `#${value}` : undefined;
}
function paragraphs(element: Element, theme: Map<string, string>): PreviewParagraph[] {
  const body = child(element, "txBody");
  if (!body) return [];
  return children(body, "p").map((p) => ({
    align: child(p, "pPr")?.getAttribute("algn") === "ctr" ? "center" : child(p, "pPr")?.getAttribute("algn") === "r" ? "right" : "left",
    runs: children(p).filter((run) => ["r", "fld", "br"].includes(run.localName ?? "")).map((run) => {
      const props = child(run, "rPr"); const defaults = child(child(p, "pPr"), "defRPr");
      const attr = (name: string) => props?.getAttribute(name) ?? defaults?.getAttribute(name);
      const runText = run.localName === "br" ? "\n" : text(run);
      const fontTypes = /[\u3000-\u9fff\uac00-\ud7af]/u.test(runText) ? ["ea", "latin"] : ["latin", "ea"];
      const fontFamily = fontTypes.map((name) => child(props, name)?.getAttribute("typeface") || child(defaults, name)?.getAttribute("typeface")).find(Boolean) ?? undefined;
      return { text: runText, bold: attr("b") === "1", italic: attr("i") === "1", underline: attr("u") === "sng", size: attr("sz") ? Number(attr("sz")) / 100 : undefined, color: color(child(props, "solidFill") ?? child(defaults, "solidFill"), theme), fontFamily };
    }),
  }));
}
function pptx(pkg: Package): DocumentPreviewModel {
  const presentation = xml(pkg, "ppt/presentation.xml");
  const size = elements(presentation, "sldSz")[0];
  const width = num(size, "cx", 9144000); const height = num(size, "cy", 6858000);
  if (width <= 0 || height <= 0) throw new Error("preview_package_invalid");
  const ordered = elements(presentation, "sldId");
  if (ordered.length > 500) throw new Error("preview_package_limits");
  const rels = relationships(pkg, "ppt/presentation.xml");
  const slides: PreviewSlide[] = [];
  let count = 0;
  for (const [index, slideId] of ordered.entries()) {
    const part = rels.get(slideId.getAttribute("r:id") ?? "")?.target;
    if (!part) throw new Error("preview_package_invalid");
    const doc = xml(pkg, part); const slideRels = relationships(pkg, part);
    const objects: SlideObject[] = [];
    const layoutPart = [...slideRels.values()].find((rel) => rel.type.endsWith("/slideLayout"))?.target;
    const layout = layoutPart && pkg[layoutPart] ? xml(pkg, layoutPart) : null;
    const masterPart = layoutPart ? [...relationships(pkg, layoutPart).values()].find((rel) => rel.type.endsWith("/slideMaster"))?.target : undefined;
    const master = masterPart && pkg[masterPart] ? xml(pkg, masterPart) : null;
    const themePart = masterPart ? [...relationships(pkg, masterPart).values()].find((rel) => rel.type.endsWith("/theme"))?.target : undefined;
    const theme = new Map<string, string>();
    if (themePart && pkg[themePart]) for (const item of children(elements(xml(pkg, themePart), "clrScheme")[0])) {
      const value = elements(item, "srgbClr")[0]?.getAttribute("val") ?? elements(item, "sysClr")[0]?.getAttribute("lastClr");
      if (value) theme.set(item.localName ?? "", value);
    }
    const mapping = master ? elements(master, "clrMap")[0] : undefined;
    for (const [alias, standard] of [["bg1", "lt1"], ["tx1", "dk1"], ["bg2", "lt2"], ["tx2", "dk2"]]) {
      const mapped = theme.get(mapping?.getAttribute(alias) ?? standard);
      if (mapped) theme.set(alias, mapped);
    }
    const phShape = (shape: Element, source: Document | null): Element | undefined => {
      const ph = elements(shape, "ph")[0];
      if (!ph || !source) return undefined;
      return elements(source, "sp").find((candidate) => {
        const c = elements(candidate, "ph")[0];
        return c && (ph.hasAttribute("idx") ? c.getAttribute("idx") === ph.getAttribute("idx") : (c.getAttribute("type") ?? "obj") === (ph.getAttribute("type") ?? "obj"));
      });
    };
    const walk = (tree: Element, transform = { sx: 1, sy: 1, tx: 0, ty: 0 }, depth = 0) => {
      if (depth > 32) throw new Error("preview_package_limits");
      for (const shape of children(tree)) {
        const tag = shape.localName;
        if (!["sp", "pic", "graphicFrame", "cxnSp", "grpSp"].includes(tag ?? "")) continue;
        if (++count > MAX_OBJECTS) throw new Error("preview_package_limits");
        const props = child(shape, tag === "grpSp" ? "grpSpPr" : "spPr");
        const inherited = phShape(shape, layout) ?? phShape(shape, master);
        const xfrm = child(props, "xfrm") ?? child(shape, "xfrm") ?? (inherited ? elements(inherited, "xfrm")[0] : undefined);
        const off = child(xfrm, "off"); const ext = child(xfrm, "ext");
        if (tag === "grpSp") {
          const chOff = child(xfrm, "chOff"); const chExt = child(xfrm, "chExt");
          const sx = num(ext, "cx", 1) / Math.max(1, num(chExt, "cx", 1)); const sy = num(ext, "cy", 1) / Math.max(1, num(chExt, "cy", 1));
          walk(shape, { sx: transform.sx * sx, sy: transform.sy * sy, tx: transform.tx + transform.sx * (num(off, "x") - num(chOff, "x") * sx), ty: transform.ty + transform.sy * (num(off, "y") - num(chOff, "y") * sy) }, depth + 1); continue;
        }
        const geometry: SlideGeometry = { x: transform.tx + num(off, "x") * transform.sx, y: transform.ty + num(off, "y") * transform.sy, width: num(ext, "cx") * transform.sx, height: num(ext, "cy") * transform.sy, rotation: num(xfrm, "rot") / 60000 };
        const id = `${part}:${elements(shape, "cNvPr")[0]?.getAttribute("id") ?? count}`;
        if (tag === "pic") {
          const blip = elements(shape, "blip")[0]; const image = dataImage(pkg, slideRels.get(blip?.getAttribute("r:embed") ?? "")?.target);
          const crop = elements(shape, "srcRect")[0];
          objects.push({ id, geometry, kind: image ? "image" : "placeholder", image, placeholder: "不支持的图片", crop: { left: num(crop, "l") / 100000, right: num(crop, "r") / 100000, top: num(crop, "t") / 100000, bottom: num(crop, "b") / 100000 } }); continue;
        }
        if (tag === "graphicFrame") {
          const table = elements(shape, "tbl")[0];
          if (table) objects.push({ id, geometry, kind: "table", rows: children(table, "tr").map((row) => children(row, "tc").map((cell) => text(cell))), cellParagraphs: children(table, "tr").map((row) => children(row, "tc").map((cell) => paragraphs(cell, theme))) });
          else objects.push({ id, geometry, kind: "placeholder", placeholder: elements(shape, "chart").length ? "图表" : elements(shape, "relIds").length ? "SmartArt" : "未支持的对象" });
          continue;
        }
        const prst = child(props, "prstGeom")?.getAttribute("prst") ?? (tag === "cxnSp" ? "line" : "rect");
        const lines = paragraphs(shape, theme);
        objects.push({ id, geometry, kind: lines.length ? "text" : "shape", paragraphs: lines, shape: prst, fill: child(props, "noFill") ? "none" : color(child(props, "solidFill"), theme) ?? (child(shape, "style") ? color(child(child(shape, "style"), "fillRef"), theme) : undefined), stroke: color(child(child(props, "ln"), "solidFill"), theme) });
      }
    };
    const tree = elements(doc, "spTree")[0];
    if (!tree) throw new Error("preview_package_invalid");
    walk(tree);
    slides.push({ id: slideId.getAttribute("id") ?? String(index + 1), objects, background: color(elements(doc, "bgPr")[0], theme) });
  }
  return { format: "pptx", width, height, slides };
}

function address(value: string): { row: number; column: number } {
  const match = /^([A-Z]{1,3})([1-9]\d{0,6})$/i.exec(value);
  if (!match) throw new Error("preview_package_invalid");
  const column = [...match[1].toUpperCase()].reduce((sum, char) => sum * 26 + char.charCodeAt(0) - 64, 0);
  const row = Number(match[2]);
  if (column > 16384 || row > 1048576) throw new Error("preview_package_limits");
  return { row, column };
}
function xlsx(pkg: Package): DocumentPreviewModel {
  const book = xml(pkg, "xl/workbook.xml"); const rels = relationships(pkg, "xl/workbook.xml");
  const strings = pkg["xl/sharedStrings.xml"] ? elements(xml(pkg, "xl/sharedStrings.xml"), "si").map(text) : [];
  const sheets: PreviewSheet[] = []; let count = 0;
  for (const entry of elements(book, "sheet")) {
    const part = rels.get(entry.getAttribute("r:id") ?? "")?.target;
    if (!part) throw new Error("preview_package_invalid");
    const doc = xml(pkg, part); let rowCount = 1; let columnCount = 1;
    const cells = elements(doc, "c").map((cell) => {
      if (++count > MAX_OBJECTS) throw new Error("preview_package_limits");
      const cellAddress = cell.getAttribute("r") ?? ""; const position = address(cellAddress);
      rowCount = Math.max(rowCount, position.row); columnCount = Math.max(columnCount, position.column);
      const raw = child(cell, "v")?.textContent ?? ""; const type = cell.getAttribute("t");
      const value = type === "s" ? strings[Number(raw)] : type === "inlineStr" ? text(cell) : type === "b" ? raw === "1" ? "TRUE" : "FALSE" : raw;
      if (value === undefined) throw new Error("preview_package_invalid");
      return { address: cellAddress, ...position, value, formula: child(cell, "f")?.textContent ?? undefined };
    });
    const merges = elements(doc, "mergeCell").map((item) => {
      const [start, end] = (item.getAttribute("ref") ?? "").split(":"); const a = address(start); const b = address(end ?? start);
      rowCount = Math.max(rowCount, b.row); columnCount = Math.max(columnCount, b.column);
      return { startRow: a.row, startColumn: a.column, endRow: b.row, endColumn: b.column };
    });
    sheets.push({ id: entry.getAttribute("sheetId") ?? String(sheets.length), name: entry.getAttribute("name") ?? "", cells, rowCount, columnCount, merges });
  }
  const calculation = elements(book, "calcPr")[0];
  return { format: "xlsx", sheets, calculationState: calculation?.getAttribute("fullCalcOnLoad") === "1" || calculation?.getAttribute("forceFullCalc") === "1" ? "stale" : "cached" };
}
function xmind(pkg: Package): DocumentPreviewModel {
  let count = 0;
  if (pkg["content.json"]) {
    if (pkg["content.json"].byteLength > MAX_XML) throw new Error("preview_package_limits");
    const value: unknown = JSON.parse(decoder.decode(pkg["content.json"]));
    if (!Array.isArray(value) || value.length > 500) throw new Error("preview_package_invalid");
    const topic = (raw: unknown, depth: number): PreviewTopic => {
      if (depth > 100 || ++count > MAX_OBJECTS) throw new Error("preview_package_limits");
      if (!raw || typeof raw !== "object") throw new Error("preview_package_invalid");
      const item = raw as Record<string, any>;
      if (typeof item.id !== "string" || typeof item.title !== "string") throw new Error("preview_package_invalid");
      const attached = item.children?.attached ?? []; const detached = item.children?.detached ?? [];
      if (!Array.isArray(attached) || !Array.isArray(detached)) throw new Error("preview_package_invalid");
      return { id: item.id, title: item.title, notes: typeof item.notes?.plain?.content === "string" ? item.notes.plain.content : undefined, children: [...attached, ...detached].map((child) => topic(child, depth + 1)) };
    };
    return { format: "xmind", sheets: value.map((sheet) => ({ id: String(sheet.id), title: String(sheet.title ?? ""), root: topic(sheet.rootTopic, 0) })) };
  }
  if (!pkg["content.xml"]) throw new Error("preview_xmind_format_unsupported");
  const doc = xml(pkg, "content.xml");
  const topic = (item: Element, depth: number): PreviewTopic => {
    if (depth > 100 || ++count > MAX_OBJECTS) throw new Error("preview_package_limits");
    return { id: item.getAttribute("id") ?? String(count), title: child(item, "title")?.textContent ?? "", notes: child(child(item, "notes"), "plain")?.textContent ?? undefined, children: children(child(item, "children"), "topics").flatMap((group) => children(group, "topic").map((entry) => topic(entry, depth + 1))) };
  };
  return { format: "xmind", sheets: elements(doc, "sheet").map((sheet) => { const root = child(sheet, "topic"); if (!root) throw new Error("preview_package_invalid"); return { id: sheet.getAttribute("id") ?? "", title: child(sheet, "title")?.textContent ?? "", root: topic(root, 0) }; }) };
}
