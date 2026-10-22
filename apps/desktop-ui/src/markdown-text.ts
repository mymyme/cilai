import { unified } from "unified";
import remarkParse from "remark-parse";
import remarkGfm from "remark-gfm";

const parser = unified().use(remarkParse).use(remarkGfm);

type TextNode = { type: string; value?: string; alt?: string | null; children?: TextNode[] };

export function markdownText(source: string): string {
  function text(node: TextNode): string {
    if (node.type === "html") return "";
    if (node.type === "image") return node.alt ?? "";
    if (node.value !== undefined) return node.value;
    const separator = ["root", "blockquote", "list", "listItem", "table", "tableRow"].includes(node.type) ? " " : "";
    return (node.children ?? []).map(text).join(separator);
  }
  return text(parser.parse(source));
}
