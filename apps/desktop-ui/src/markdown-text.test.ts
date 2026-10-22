import { expect, test } from "vitest";
import { markdownText } from "./markdown-text";

test("摘要与 Markdown 正文使用相同语法，不产生残缺 HTML 标签", () => {
  expect(markdownText("答案\n</think>\n\n答案").replace(/\s+/g, " ").trim()).toBe("答案 答案");
  expect(markdownText("# 标题\n\n> **结论** [资料](https://example.com)")).toBe("标题 结论 资料");
});

test("摘要保留代码、比较符号和正文重复", () => {
  expect(markdownText("标签 `</think>` 与 x > 1")).toBe("标签 </think> 与 x > 1");
  expect(markdownText("```xml\n</think>\n```")).toBe("</think>");
  expect(markdownText("回答\n\n回答")).toBe("回答 回答");
});
