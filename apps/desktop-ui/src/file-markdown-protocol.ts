import type { MarkdownPage } from "./file-markdown-model";

export type FileMarkdownRequest =
  | { type: "load"; requestId: number; content: string }
  | { type: "page"; requestId: number; index: number; expand: boolean };
export type FileMarkdownResponse =
  | { type: "page"; requestId: number; page: MarkdownPage }
  | { type: "error"; requestId: number };
