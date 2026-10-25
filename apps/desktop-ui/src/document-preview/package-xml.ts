import { DOMParser, type Element, type Document } from "@xmldom/xmldom";
import type { PreviewParagraph } from "./model";
export type Package = Record<string, Uint8Array>;
const decoder = new TextDecoder("utf-8", { fatal: true });

export function xml(pkg: Package, name: string): Document {
  const value = pkg[name];
  if (!value) throw new Error("preview_package_invalid");
  const doc = new DOMParser({ onError: (level) => { if (level !== "warning") throw new Error("preview_package_invalid"); } }).parseFromString(decoder.decode(value), "text/xml");
  if (!doc.documentElement || doc.getElementsByTagName("parsererror").length > 0) throw new Error("preview_package_invalid");
  return doc;
}
export function elements(parent: Document | Element, name: string): Element[] {
  return Array.from(parent.getElementsByTagName("*")).filter((element) => element.localName === name);
}
export function child(parent: Element | Document | undefined, name: string): Element | undefined {
  if (!parent) return undefined;
  return Array.from(parent.childNodes).find((node) => node.nodeType === 1 && (node as Element).localName === name) as Element | undefined;
}
export function children(parent: Element | Document | undefined, name?: string): Element[] {
  if (!parent) return [];
  return Array.from(parent.childNodes).filter((node) => node.nodeType === 1 && (!name || (node as Element).localName === name)) as Element[];
}
export function num(element: Element | undefined, name: string, defaultValue = 0): number {
  const value = Number(element?.getAttribute(name) ?? defaultValue);
  if (!Number.isFinite(value)) throw new Error("preview_package_invalid");
  return value;
}
export function resolvePart(base: string, target: string): string {
  if (!target || target.includes("\\") || /^[a-z]+:/i.test(target)) throw new Error("preview_package_unsafe");
  const parts = target.startsWith("/") ? [] : base.split("/").slice(0, -1);
  for (const part of target.split("/")) {
    if (part === "..") { if (!parts.length) throw new Error("preview_package_unsafe"); parts.pop(); }
    else if (part && part !== ".") parts.push(part);
  }
  return parts.join("/");
}
export function relationships(pkg: Package, part: string): Map<string, { target: string; type: string }> {
  const split = part.lastIndexOf("/");
  const relPath = `${part.slice(0, split + 1)}_rels/${part.slice(split + 1)}.rels`;
  if (!pkg[relPath]) return new Map();
  return new Map(elements(xml(pkg, relPath), "Relationship").filter((rel) => rel.getAttribute("TargetMode") !== "External").map((rel) => [rel.getAttribute("Id") ?? "", { target: resolvePart(part, rel.getAttribute("Target") ?? ""), type: rel.getAttribute("Type") ?? "" }]));
}
export function text(parent: Element): string { return elements(parent, "t").map((item) => item.textContent ?? "").join(""); }
export function dataImage(pkg: Package, target: string | undefined): string | undefined {
  if (!target || !pkg[target]) return undefined;
  const extension = target.split(".").pop()?.toLowerCase();
  const mime = extension === "png" ? "image/png" : extension === "jpg" || extension === "jpeg" ? "image/jpeg" : extension === "gif" ? "image/gif" : extension === "webp" ? "image/webp" : null;
  if (!mime) return undefined;
  const bytes = pkg[target];
  let binary = "";
  for (let i = 0; i < bytes.length; i += 8192) binary += String.fromCharCode(...bytes.subarray(i, i + 8192));
  return `data:${mime};base64,${btoa(binary)}`;
}
export function color(parent: Element | undefined, theme: Map<string, string>): string | undefined {
  if (!parent) return undefined;
  const value = elements(parent, "srgbClr")[0]?.getAttribute("val") ?? theme.get(elements(parent, "schemeClr")[0]?.getAttribute("val") ?? "") ?? elements(parent, "sysClr")[0]?.getAttribute("lastClr");
  return value && /^[\da-f]{6}$/i.test(value) ? `#${value}` : undefined;
}
export function paragraphs(element: Element, theme: Map<string, string>): PreviewParagraph[] {
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
