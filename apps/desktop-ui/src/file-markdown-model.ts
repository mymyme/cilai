import type { Element, Root, RootContent, Text } from "hast";
import { unified } from "unified";
import remarkParse from "remark-parse";
import remarkGfm from "remark-gfm";
import remarkRehype from "remark-rehype";
import { defaultUrlTransform } from "react-markdown";
import { splitFrontmatter } from "./markdown-frontmatter";

export const MARKDOWN_PAGE_BUDGET = { nodes: 2000, text: 16_384 } as const;
export type MarkdownBudget = { nodes: number; text: number };
export type MarkdownOversize = { label: string; nodes: number; text: number; startLine?: number; endLine?: number };
export type MarkdownPage = { index: number; pageCount: number; tree: Root | null; oversize: MarkdownOversize | null };
type Part = { node: RootContent; oversize: MarkdownOversize | null };
type StoredPage = { tree: Root; oversize: MarkdownOversize | null };
export type MarkdownDocument = { tree: Root; pages: StoredPage[] };
const root = (children: RootContent[]): Root => ({ type: "root", children });
const element = (tagName: string, children: Element["children"], properties: Element["properties"] = {}): Element => ({ type: "element", tagName, properties, children });
const text = (value: string): Text => ({ type: "text", value });
const fits = (cost: MarkdownBudget, budget: MarkdownBudget) => cost.nodes <= budget.nodes && cost.text <= budget.text;
const add = (left: MarkdownBudget, right: MarkdownBudget): MarkdownBudget => ({ nodes: left.nodes + right.nodes, text: left.text + right.text });
const ownCost = (node: Root | RootContent): MarkdownBudget => {
  const propertyText = node.type === "element" ? Object.values(node.properties).reduce<number>((total, value) => total
    + (typeof value === "string" ? value.length : Array.isArray(value) ? value.reduce<number>((length, part) => length + String(part).length, 0) : 0), 0) : 0;
  return node.type === "element" && node.tagName === "img"
    ? { nodes: 2, text: propertyText + `[图片：${String(node.properties.alt ?? "无说明")}]`.length }
    : { nodes: node.type === "element" && node.tagName === "table" ? 2 : 1, text: propertyText + (node.type === "text" ? node.value.length : 0) };
};

export function markdownTreeCost(node: Root | RootContent): MarkdownBudget {
  const cost = { nodes: 0, text: 0 };
  const stack: Array<Root | RootContent> = [node];
  while (stack.length) {
    const current = stack.pop()!;
    const currentCost = ownCost(current);
    cost.nodes += currentCost.nodes;
    cost.text += currentCost.text;
    if ("children" in current) for (const child of current.children) stack.push(child);
  }
  return cost;
}

export function parseMarkdownDocument(source: string, budget: MarkdownBudget = MARKDOWN_PAGE_BUDGET): MarkdownDocument {
  if (!Number.isInteger(budget.nodes) || budget.nodes < 8 || !Number.isInteger(budget.text) || budget.text < 2) throw new Error("markdown_invalid_budget");
  const { frontmatter, body } = splitFrontmatter(source);
  const prefixLength = source.length - body.length;
  const lineOffset = source.slice(0, prefixLength).split("\n").length - 1;
  const processor = unified().use(remarkParse).use(remarkGfm).use(remarkRehype);
  const tree = processor.runSync(processor.parse(body)) as Root;
  if (frontmatter) tree.children.unshift(element("dl", frontmatter.flatMap(([key, value]) => [element("dt", [text(key)]), element("dd", [text(value)])]), { className: ["markdown-frontmatter"], ariaLabel: "文档属性" }));
  const stack: Array<Root | RootContent> = [tree];
  while (stack.length) {
    const node = stack.pop()!;
    if (node.position) {
      const { start, end } = node.position;
      node.position = {
        start: { ...start, line: start.line + lineOffset, ...(start.offset === undefined ? {} : { offset: start.offset + prefixLength }) },
        end: { ...end, line: end.line + lineOffset, ...(end.offset === undefined ? {} : { offset: end.offset + prefixLength }) },
      };
    }
    if (node.type === "element") {
      for (const property of ["href", "src"]) {
        if (property in node.properties) node.properties[property] = defaultUrlTransform(String(node.properties[property] ?? ""));
      }
    }
    if ("children" in node) for (const child of node.children) stack.push(child);
  }

  const costs = new WeakMap<object, MarkdownBudget>();
  const costOf = (node: RootContent): MarkdownBudget => {
    const cached = costs.get(node);
    if (cached) return cached;
    let cost = ownCost(node);
    if ("children" in node) for (const child of node.children) cost = add(cost, costOf(child));
    costs.set(node, cost);
    return cost;
  };
  const exceptional = (node: RootContent, label: string): Part => ({ node, oversize: { label, ...costOf(node), startLine: node.position?.start.line, endLine: node.position?.end.line } });

  function grouped(units: RootContent[], wrap: (children: RootContent[], start: number) => Element, available: MarkdownBudget, label: string): Part[] {
    const result: Part[] = [];
    let current: RootContent[] = [];
    let start = 0;
    let used = costOf(wrap([], 0));
    for (let index = 0; index < units.length; index += 1) {
      const unit = units[index];
      const unitCost = costOf(unit);
      if (fits(add(used, unitCost), available)) { current.push(unit); used = add(used, unitCost); continue; }
      if (current.length) result.push({ node: wrap(current, start), oversize: null });
      current = [];
      start = index;
      used = costOf(wrap([], index));
      const single = wrap([unit], index);
      if (fits(add(used, unitCost), available)) { current = [unit]; used = add(used, unitCost); }
      else {
        result.push(exceptional(single, `${label} ${index + 1}`));
        start = index + 1;
      }
    }
    if (current.length) result.push({ node: wrap(current, start), oversize: null });
    return result;
  }

  function split(node: RootContent, available: MarkdownBudget): Part[] {
    if (fits(costOf(node), available)) return [{ node, oversize: null }];
    if (node.type !== "element") return [exceptional(node, "较大正文")];
    if (node.tagName === "table") {
      const head = node.children.find((child): child is Element => child.type === "element" && child.tagName === "thead");
      const body = node.children.find((child): child is Element => child.type === "element" && child.tagName === "tbody");
      if (!body) return [exceptional(node, "较大表格")];
      const wrap = (children: RootContent[]): Element => ({ ...node, children: [...(head ? [head] : []), { ...body, children: children as Element["children"] }] });
      if (!fits(costOf(wrap([])), available)) return [exceptional(node, "较大表格")];
      const rows = body.children.filter((child) => child.type === "element" && child.tagName === "tr");
      return grouped(rows, wrap, available, "较大表格行");
    }
    if (node.tagName === "ol" || node.tagName === "ul") {
      const items = node.children.filter((child) => child.type === "element" && child.tagName === "li");
      const first = Number(node.properties.start ?? 1);
      return grouped(items, (children, start) => ({ ...node, properties: { ...node.properties, ...(node.tagName === "ol" ? { start: first + start } : {}) }, children: children as Element["children"] }), available, "较大列表项");
    }
    if (node.tagName === "pre") {
      const code = node.children[0];
      if (code?.type !== "element" || code.tagName !== "code" || code.children.length !== 1 || code.children[0].type !== "text") return [exceptional(node, "较大代码块")];
      if (Array.isArray(code.properties.className) && code.properties.className.includes("language-mermaid")) return [exceptional(node, "较大图表")];
      const value = code.children[0].value;
      const wrap = (content: string): Element => ({ ...node, children: [{ ...code, children: [text(content)] }] });
      if (available.nodes < 3) return [exceptional(node, "较大代码块")];
      const textCapacity = available.text - costOf(wrap("")).text;
      if (textCapacity < 1) return [exceptional(node, "较大代码块")];
      const result: Part[] = [];
      let offset = 0;
      let start = 0;
      let length = 0;
      let line = 1;
      while (offset < value.length) {
        const newline = value.indexOf("\n", offset);
        const end = newline < 0 ? value.length : newline + 1;
        const lineLength = end - offset;
        if (length && length + lineLength > textCapacity) { result.push({ node: wrap(value.slice(start, offset)), oversize: null }); start = offset; length = 0; }
        if (lineLength > textCapacity) { result.push(exceptional(wrap(value.slice(offset, end)), `较长代码行 ${line}`)); start = end; }
        else length += lineLength;
        offset = end;
        line += 1;
      }
      if (length) result.push({ node: wrap(value.slice(start)), oversize: null });
      return result;
    }
    if (node.tagName === "section" && node.properties.dataFootnotes) {
      const heading = node.children.find((child) => child.type === "element" && child.tagName === "h2");
      const list = node.children.find((child) => child.type === "element" && child.tagName === "ol");
      if (heading && list) {
        const headCost = add(costOf(heading), ownCost(node));
        const remaining = { nodes: available.nodes - headCost.nodes, text: available.text - headCost.text };
        if (remaining.nodes < 3 || remaining.text < 1) return [exceptional(node, "较大脚注")];
        return split(list, remaining).map((part) => {
          const wrapped: Element = { ...node, children: [heading, part.node as Element["children"][number]] };
          return { node: wrapped, oversize: part.oversize ? { ...part.oversize, ...costOf(wrapped) } : null };
        });
      }
    }
    if (["blockquote", "section", "div"].includes(node.tagName) && available.nodes > 1) {
      const wrapperCost = ownCost(node);
      const remaining = { nodes: available.nodes - wrapperCost.nodes, text: available.text - wrapperCost.text };
      if (remaining.text < 1) return [exceptional(node, "较大正文")];
      const result: Part[] = [];
      for (const child of node.children) {
        for (const part of split(child, remaining)) {
          const wrapped: Element = { ...node, children: [part.node as Element["children"][number]] };
          result.push({ node: wrapped, oversize: part.oversize ? { ...part.oversize, ...costOf(wrapped) } : null });
        }
      }
      return result;
    }
    return [exceptional(node, "较大段落")];
  }

  const pages: StoredPage[] = [];
  let children: RootContent[] = [];
  let used = { nodes: 1, text: 0 };
  const flush = () => {
    if (children.length) pages.push({ tree: root(children), oversize: null });
    children = [];
    used = { nodes: 1, text: 0 };
  };
  for (const child of tree.children) {
    for (const part of split(child, { nodes: budget.nodes - 1, text: budget.text })) {
      if (part.oversize) { flush(); pages.push({ tree: root([part.node]), oversize: { ...part.oversize, nodes: part.oversize.nodes + 1 } }); continue; }
      const cost = costOf(part.node);
      if (!fits(add(used, cost), budget)) flush();
      children.push(part.node);
      used = add(used, cost);
    }
  }
  flush();
  if (!pages.length) pages.push({ tree: root([]), oversize: null });
  return { tree, pages };
}

export function readMarkdownPage(document: MarkdownDocument, index: number, expand = false): MarkdownPage {
  if (!Number.isInteger(index) || index < 0 || index >= document.pages.length) throw new Error("markdown_invalid_page");
  const page = document.pages[index];
  return { index, pageCount: document.pages.length, tree: page.oversize && !expand ? null : page.tree, oversize: page.oversize };
}
