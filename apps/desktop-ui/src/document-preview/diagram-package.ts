import type { Element } from "@xmldom/xmldom";
import { child, children, elements, num, relationships, xml, type Package } from "./package-xml";

export function readDiagram(pkg: Package, relIds: Element, slideRels: ReturnType<typeof relationships>) {
  const dataPart = slideRels.get(relIds.getAttribute("r:dm") ?? "")?.target;
  if (!dataPart) throw new Error("SmartArt 数据关系无效。");
  const data = xml(pkg, dataPart); const rels = relationships(pkg, dataPart);
  const extension = elements(data, "dataModelExt")[0];
  const drawingPart = rels.get(extension?.getAttribute("relId") ?? "")?.target
    ?? [...rels.values()].find(rel => rel.type.endsWith("/diagramDrawing"))?.target;
  if (!drawingPart) throw new Error("此 SmartArt 未保存已计算的图形布局，无法准确显示。");
  const drawing = xml(pkg, drawingPart); const tree = elements(drawing, "spTree")[0];
  if (!tree) throw new Error("SmartArt 图形布局无效。");
  const transform = child(child(tree, "grpSpPr"), "xfrm");
  const extent = child(transform, "chExt") ?? child(transform, "ext");
  const offset = child(transform, "chOff") ?? child(transform, "off");
  let viewport = { x: num(offset, "x"), y: num(offset, "y"), width: num(extent, "cx"), height: num(extent, "cy") };
  if (viewport.width <= 0 || viewport.height <= 0) {
    const bounds = children(tree).filter(shape => ["sp", "cxnSp", "pic", "grpSp"].includes(shape.localName ?? "")).map(shape => {
      const xfrm = child(child(shape, shape.localName === "grpSp" ? "grpSpPr" : "spPr"), "xfrm");
      return { x: num(child(xfrm, "off"), "x"), y: num(child(xfrm, "off"), "y"), width: num(child(xfrm, "ext"), "cx"), height: num(child(xfrm, "ext"), "cy") };
    }).filter(bound => bound.width > 0 && bound.height > 0);
    if (!bounds.length) throw new Error("SmartArt 图形尺寸无效。");
    const x = Math.min(...bounds.map(bound => bound.x)); const y = Math.min(...bounds.map(bound => bound.y));
    viewport = { x, y, width: Math.max(...bounds.map(bound => bound.x + bound.width)) - x, height: Math.max(...bounds.map(bound => bound.y + bound.height)) - y };
  }
  return { part: drawingPart, tree, rels: relationships(pkg, drawingPart), viewport };
}
