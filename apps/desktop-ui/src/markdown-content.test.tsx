import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { MarkdownContent, splitFrontmatter } from "./markdown-content";

afterEach(cleanup);

test.each(["D:/工作区/报告.md", "D:\\工作区\\报告.md", "file:///D:/工作区/报告.md", "file:///Users/me/报告.md"])("已授权本地链接不会被 Markdown 协议过滤：%s", (path) => {
  const onOpenLink = vi.fn();
  const onContextLink = vi.fn();
  render(<MarkdownContent canOpenLink={() => true} onOpenLink={onOpenLink} onContextLink={onContextLink}>{`[报告](${path})`}</MarkdownContent>);
  const link = screen.getByRole("link", { name: "报告" });
  fireEvent.click(link);
  expect(decodeURI(onOpenLink.mock.calls[0][0])).toBe(path);
  fireEvent.contextMenu(link);
  expect(onContextLink).toHaveBeenCalled();
});

test("脚本和未授权文件仍不可点击", () => {
  render(<MarkdownContent canOpenLink={() => false}>{"[脚本](javascript:alert%281%29) [外部文件](file:///D:/other/private.txt)"}</MarkdownContent>);
  expect(screen.queryAllByRole("link")).toHaveLength(0);
});

test("调用方允许打开也不会绕过脚本协议过滤", () => {
  render(<MarkdownContent canOpenLink={() => true}>{"[脚本](javascript:alert%281%29)"}</MarkdownContent>);
  expect(screen.queryAllByRole("link")).toHaveLength(0);
});

test("流式正文更新时保留代码块的横向阅读位置", () => {
  const code = '```json\n{"name":"very-long-name","description":"a long description that extends beyond the visible width"}\n```';
  const view = render(<MarkdownContent canOpenLink={() => false} onOpenLink={() => undefined}>{code}</MarkdownContent>);
  const block = view.container.querySelector("pre")!;
  block.scrollLeft = 120;

  view.rerender(<MarkdownContent canOpenLink={() => false} onOpenLink={() => undefined}>{`${code}\n\n后续正文`}</MarkdownContent>);

  expect(view.container.querySelector("pre")).toBe(block);
  expect(block.scrollLeft).toBe(120);

  view.rerender(<MarkdownContent canOpenLink={() => false} onOpenLink={() => undefined}>{`${code.replace("a long description", "a much longer description")}\n\n后续正文`}</MarkdownContent>);
  expect(view.container.querySelector("pre")).toBe(block);
  expect(block.scrollLeft).toBe(120);
});

test("链接回调更新后仍使用最新的打开行为", () => {
  const first = vi.fn();
  const second = vi.fn();
  const view = render(<MarkdownContent canOpenLink={() => true} onOpenLink={first}>[文件](/readme.md)</MarkdownContent>);
  view.rerender(<MarkdownContent canOpenLink={() => true} onOpenLink={second}>[文件](/readme.md)</MarkdownContent>);
  fireEvent.click(view.getByRole("link", { name: "文件" }));
  expect(first).not.toHaveBeenCalled();
  expect(second).toHaveBeenCalledWith("/readme.md");
});

test("可打开的文件链接会传递右键事件和路径", () => {
  const onContextLink = vi.fn();
  render(<MarkdownContent canOpenLink={() => true} onContextLink={onContextLink}>[图片](产物/图片.png)</MarkdownContent>);
  const link = screen.getByRole("link", { name: "图片" });
  fireEvent.contextMenu(link);
  expect(onContextLink).toHaveBeenCalledWith(expect.anything(), "%E4%BA%A7%E7%89%A9/%E5%9B%BE%E7%89%87.png");
});

test("开头的属性块渲染为文档属性，不再变成一大段标题", () => {
  const source = '---\nname: "访谈纪要整理"\ndescription: 整理访谈\nmetadata:\n  tags: a, b\n---\n\n# 正文标题\n';
  const { container } = render(<MarkdownContent>{source}</MarkdownContent>);
  expect(container.querySelector("dl.markdown-frontmatter")?.getAttribute("aria-label")).toBe("文档属性");
  expect(screen.getByText("访谈纪要整理")).toBeTruthy();
  expect(screen.getByText("tags: a, b")).toBeTruthy();
  expect(screen.getAllByRole("heading").map((heading) => heading.textContent)).toEqual(["正文标题"]);
});

test("不以属性块开头或块内没有键时原样交给 Markdown", () => {
  expect(splitFrontmatter("正文\n---\n").frontmatter).toBeNull();
  expect(splitFrontmatter("---\n仅一行说明\n---\n正文").frontmatter).toBeNull();
  expect(splitFrontmatter("---\nversion: 1.0\n---\n正文")).toEqual({ frontmatter: [["version", "1.0"]], body: "正文" });
});
