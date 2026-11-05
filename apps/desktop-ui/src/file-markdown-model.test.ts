import { expect, test } from "vitest";
import type { Element, Root, RootContent } from "hast";
import { MARKDOWN_PAGE_BUDGET, markdownTreeCost, parseMarkdownDocument, readMarkdownPage, type MarkdownBudget, type MarkdownDocument } from "./file-markdown-model";
import { splitFrontmatter } from "./markdown-frontmatter";

function elements(root: Root | RootContent, tag: string): Element[] {
  const result: Element[] = [];
  const pending: Array<Root | RootContent> = [root];
  while (pending.length) {
    const node = pending.shift()!;
    if (node.type === "element" && node.tagName === tag) result.push(node);
    if ("children" in node) pending.unshift(...node.children);
  }
  return result;
}
function content(node: Root | RootContent): string {
  return node.type === "text" ? node.value : "children" in node ? node.children.map(content).join("") : "";
}
function normalPages(document: MarkdownDocument, budget: MarkdownBudget = MARKDOWN_PAGE_BUDGET) {
  for (let index = 0; index < document.pages.length; index += 1) {
    const page = readMarkdownPage(document, index);
    if (!page.tree) continue;
    const cost = markdownTreeCost(page.tree);
    expect(cost.nodes).toBeLessThanOrEqual(budget.nodes);
    expect(cost.text).toBeLessThanOrEqual(budget.text);
  }
}

test.each([{ nodes: 80, text: 16_384 }, { nodes: 2000, text: 400 }])("表格同时受节点与文本预算约束，全部行/对齐保留 %j", (budget) => {
  const rows = Array.from({ length: 800 }, (_, i) => `| 行${i}😀 | 数值${i}\\|字 |`);
  const document = parseMarkdownDocument(`| 左列 | 右列 |\n| :--- | ---: |\n${rows.join("\n")}`, budget);
  expect(document.pages.length).toBeGreaterThan(2);
  normalPages(document, budget);
  const actual: string[][] = [];
  for (const page of document.pages) {
    expect(page.oversize).toBeNull();
    const headers = elements(page.tree, "th");
    expect(headers.map(node => node.properties.align)).toEqual(["left", "right"]);
    for (const body of elements(page.tree, "tbody")) actual.push(...elements(body, "tr").map(row => elements(row, "td").map(content)));
  }
  expect(actual).toEqual(rows.map((_, i) => [`行${i}😀`, `数值${i}|字`]));
});

test("长代码按完整行分组，拼回已解析代码内容且不破坏代理对和空行", () => {
  const code = Array.from({ length: 5000 }, (_, i) => `${i}:😀${i % 3 ? "" : "\n"}`).join("\n") + "\n";
  const document = parseMarkdownDocument(`\`\`\`typescript\n${code}\`\`\`\n`);
  normalPages(document);
  expect(document.pages.length).toBeGreaterThan(1);
  const rendered = document.pages.flatMap(page => elements(page.tree, "code")).map(content);
  expect(rendered.join("")).toBe(code);
  expect(rendered.every(page => page.endsWith("\n"))).toBe(true);
  expect(document.pages.every(page => elements(page.tree, "code")[0].properties.className?.includes("language-typescript"))).toBe(true);
});

test("巨型单行、单元格和图表明确占位，主动展开才返回完整块", () => {
  const huge = "😀".repeat(10_000);
  for (const source of [`\`\`\`text\n前行\n${huge}\n尾行\n\`\`\``, `| 列 |\n|---|\n| ${huge} |`, `\`\`\`mermaid\n${huge}\n\`\`\``]) {
    const document = parseMarkdownDocument(source);
    normalPages(document);
    const index = document.pages.findIndex(page => page.oversize);
    expect(index).toBeGreaterThanOrEqual(0);
    expect(readMarkdownPage(document, index).tree).toBeNull();
    expect(content(readMarkdownPage(document, index, true).tree!)).toContain(huge);
  }
});

test("引用中的有序列表保留起始号、条目顺序、任务勾选与嵌套", () => {
  const source = Array.from({ length: 120 }, (_, i) => `> ${i + 7}. 条目${i}\n>\n>    - [${i % 2 ? " " : "x"}] 子项${i}`).join("\n>\n");
  const budget = { nodes: 110, text: 400 };
  const document = parseMarkdownDocument(source, budget);
  normalPages(document, budget);
  const ordered = document.pages.flatMap(page => elements(page.tree, "ol"));
  let expectedStart = 7;
  for (const list of ordered) {
    expect(Number(list.properties.start ?? 1)).toBe(expectedStart);
    expectedStart += list.children.filter(child => child.type === "element" && child.tagName === "li").length;
  }
  expect(expectedStart).toBe(127);
  const checkboxes = document.pages.flatMap(page => elements(page.tree, "input"));
  expect(checkboxes).toHaveLength(120);
  expect(checkboxes.every(node => node.properties.disabled)).toBe(true);
});

test("整篇先解析文末引用与脚注，分页不会重编号或丢定义", () => {
  const source = `首处[文末引用][link]、脚注[^one]。\n\n${Array.from({ length: 100 }, (_, i) => `段落${i}文字`).join("\n\n")}\n\n尾处[^one]与[^two]。\n\n[link]: https://example.test/file\n[^one]: 第一脚注\n[^two]: 第二脚注\n`;
  const document = parseMarkdownDocument(source, { nodes: 60, text: 300 });
  normalPages(document, { nodes: 60, text: 300 });
  const fullLinks = elements(document.tree, "a").map(node => [node.properties.href, content(node)]);
  const pageLinks = document.pages.flatMap(page => elements(page.tree, "a")).map(node => [node.properties.href, content(node)]);
  expect(pageLinks).toEqual(fullLinks);
  expect(pageLinks).toContainEqual(["https://example.test/file", "文末引用"]);
  expect(pageLinks.filter(([href]) => href === "#user-content-fn-one").map(([, label]) => label)).toEqual(["1", "1"]);
});

test("文档属性计入预算，原始HTML删除，脚本URL不进入渲染树", () => {
  const source = `---\nname: 属性\n---\n\n[脚本](javascript:alert%281%29) ![图片](javascript:evil)\n\n<script>globalThis.bad=true</script>\n\n## 正文`;
  const document = parseMarkdownDocument(source);
  expect(elements(document.tree, "dl")).toHaveLength(1);
  expect(elements(document.tree, "script")).toHaveLength(0);
  expect(elements(document.tree, "a")[0].properties.href).toBe("");
  expect(elements(document.tree, "img")[0].properties.src).toBe("");
  expect(content(document.tree)).not.toContain("globalThis.bad");
  expect(elements(document.tree, "h2")[0].position?.start.line).toBe(9);
  expect(splitFrontmatter(source).body).toContain("## 正文");
  normalPages(document);
});

test("巨大图片说明不能绕过文本预算，无效页码明确拒绝", () => {
  const document = parseMarkdownDocument(`![${"图片说明".repeat(6000)}](https://example.test/image.png)`);
  expect(readMarkdownPage(document, 0).tree).toBeNull();
  expect(readMarkdownPage(document, 0).oversize!.text).toBeGreaterThan(MARKDOWN_PAGE_BUDGET.text);
  expect(() => readMarkdownPage(document, -1)).toThrow("markdown_invalid_page");
  expect(() => readMarkdownPage(document, 0.5)).toThrow("markdown_invalid_page");
  expect(() => readMarkdownPage(document, document.pages.length)).toThrow("markdown_invalid_page");
});

test("URL与标题属性不能让小节点绕过跨线程文本预算", () => {
  const source = `[短文字](https://example.test/${"a".repeat(20_000)} "标题")`;
  const document = parseMarkdownDocument(source);
  expect(readMarkdownPage(document, 0).tree).toBeNull();
  expect(elements(readMarkdownPage(document, 0, true).tree!, "a")[0].properties.href).toHaveLength("https://example.test/".length + 20_000);
});

test("脚注跨页重复标签但不重复编号或漏掉内容，标签和包装也计数", () => {
  const source = Array.from({ length: 80 }, (_, i) => `引用${i}[^note${i}]`).join("\n\n") + "\n\n"
    + Array.from({ length: 80 }, (_, i) => `[^note${i}]: 脚注正文${i}`).join("\n");
  const budget = { nodes: 80, text: 300 };
  const document = parseMarkdownDocument(source, budget);
  normalPages(document, budget);
  const sections = document.pages.flatMap(page => elements(page.tree, "section"));
  expect(sections.length).toBeGreaterThan(1);
  for (const section of sections) expect(elements(section, "h2")[0].properties.id).toBe("footnote-label");
  expect(sections.flatMap(section => elements(section, "li")).map(node => node.properties.id)).toEqual(Array.from({ length: 80 }, (_, i) => `user-content-fn-note${i}`));
});
