import { expect, test } from "vitest";
import { workspacePathsEqual, workspaceRelativePath } from "./workspace-path";

test.each([
  ["D:\\资料\\报告 A.md", "D:\\资料", "报告 A.md"],
  ["d:/资料/报告.md:12:3", "D:/资料", "报告.md"],
  ["/D:/资料/报告.md#L12", "D:/资料", "报告.md"],
  ["D:%5C资料%5C报告.md", "D:/资料", "报告.md"],
  ["D:\\B端产品\\测试文件夹\\images\\海滩.png", "\\\\?\\D:\\B端产品\\测试文件夹", "images/海滩.png"],
  ["D:/B端产品/测试文件夹/images/海滩.png", "//?/D:/B端产品/测试文件夹", "images/海滩.png"],
  ["\\\\?\\D:\\资料\\报告.md", "D:/资料", "报告.md"],
  ["%5C%5C%3F%5CD:%5C资料%5C报告.md", "D:/资料", "报告.md"],
  ["file:///D:/资料/报告.md#L12", "\\\\?\\D:\\资料", "报告.md"],
  ["\\\\SERVER\\share\\资料\\报告.md", "\\\\?\\UNC\\server\\share\\资料", "报告.md"],
  ["\\\\?\\UNC\\server\\share\\资料\\报告.md", "\\\\server\\share\\资料", "报告.md"],
  ["file:///D:/资料/报告%20A.md", "D:/资料", "报告 A.md"],
  ["file://localhost/D:/资料/报告.md", "D:/资料", "报告.md"],
  ["\\\\server\\share\\资料\\报告.md", "\\\\server\\share\\资料", "报告.md"],
  ["file://SERVER/share/资料/报告.md", "\\\\server\\share\\资料", "报告.md"],
  ["file:///Users/me/资料/报告.md", "/Users/me/资料", "报告.md"],
  ["./产物/报告.md", "/Users/me/资料", "产物/报告.md"],
  ["/报告.md", "/", "报告.md"],
])("文件地址 %s 按工作区解析", (href, root, result) => {
  expect(workspaceRelativePath(href, root)).toBe(result);
});

test.each([
  "D:/资料外/secret.md", "C:/资料/report.md", "D:report.md", "D:/资料/../secret.md",
  "file:///D:/资料/%2e%2e/secret.md", "file://other/资料/report.md", "//other/资料/report.md",
  "../secret.md", "%2e%2e%5csecret.md", "javascript:alert(1)", "https://example.com/file.md",
  "file:///D:/资料/report.md%00", "file://user@host/资料/report.md", "D:/资料/report.md:stream", "%invalid",
  "\\\\?\\D:\\资料外\\报告.md", "\\\\?\\D:\\资料\\..\\secret.md", "\\\\?\\GLOBALROOT\\Device\\secret.md",
  "\\\\.\\D:\\资料\\report.md", "\\\\?\\Volume{abc}\\资料\\report.md", "\\\\?\\UNC\\other\\share\\report.md",
])("越界或非法地址保持不可打开：%s", (href) => {
  expect(workspaceRelativePath(href, "D:/资料")).toBeNull();
});

test("规范路径前缀不放宽工作区边界", () => {
  expect(workspaceRelativePath("D:/资料外/report.md", "\\\\?\\D:\\资料")).toBeNull();
  expect(workspaceRelativePath("file:///D:/资料/%2e%2e/secret.md", "\\\\?\\D:\\资料")).toBeNull();
  expect(workspaceRelativePath("//other/share/report.md", "\\\\?\\UNC\\server\\share")).toBeNull();
  expect(workspaceRelativePath("report.md", "\\\\?\\GLOBALROOT\\Device\\disk")).toBeNull();
});

test("Windows 大小写匹配不改变文件路径，macOS 保留大小写", () => {
  expect(workspacePathsEqual("Reports\\FILE.md", "reports/file.md", "D:/work")).toBe(true);
  expect(workspacePathsEqual("Reports/FILE.md", "reports/file.md", "/Users/me/work")).toBe(false);
  expect(workspaceRelativePath("/Users/ME/work/file.md", "/Users/me/work")).toBeNull();
});
