import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { AlertDialog, ConfirmDialog } from "./confirm-dialog";

const confirmButton = () => screen.getByRole("button", { name: "确认完成" }) as HTMLButtonElement;
const base = { title: "确认阶段完成", description: "结论会写入变更历史。", confirmLabel: "确认完成", onCancel: () => {} };

afterEach(cleanup);

describe("ConfirmDialog", () => {
  it("没有 commentLabel 时不渲染说明输入框", () => {
    render(<ConfirmDialog {...base} onConfirm={() => {}}/>);
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(confirmButton().disabled).toBe(false);
  });

  it("要求填写结论时，未填写则确认按钮不可用", () => {
    render(<ConfirmDialog {...base} commentLabel="验收结论" onConfirm={() => {}}/>);
    expect(confirmButton().disabled).toBe(true);
  });

  it("只有空白字符仍视为未填写", () => {
    render(<ConfirmDialog {...base} commentLabel="验收结论" onConfirm={() => {}}/>);
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "   " } });
    expect(confirmButton().disabled).toBe(true);
  });

  it("把本次填写的结论原样交给确认回调，并去除首尾空白", () => {
    const onConfirm = vi.fn();
    render(<ConfirmDialog {...base} commentLabel="验收结论" onConfirm={onConfirm}/>);
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "  三项验收标准全部通过  " } });
    fireEvent.click(confirmButton());
    expect(onConfirm).toHaveBeenCalledWith("三项验收标准全部通过");
  });

  it("每次打开都是空白，上一次动作的结论不会带到下一个动作", () => {
    const onConfirm = vi.fn();
    const first = render(<ConfirmDialog {...base} commentLabel="验收结论" onConfirm={onConfirm}/>);
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "阶段一的结论" } });
    first.unmount();

    render(<ConfirmDialog {...base} commentLabel="返工反馈" onConfirm={onConfirm}/>);
    expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe("");
    expect(confirmButton().disabled).toBe(true);
  });

  it("处理中不接受 Esc 关闭，避免中途丢失正在提交的动作", () => {
    const onCancel = vi.fn();
    render(<ConfirmDialog {...base} busy commentLabel="验收结论" onCancel={onCancel} onConfirm={() => {}}/>);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onCancel).not.toHaveBeenCalled();
  });
});

describe("AlertDialog", () => {
  it("错误提示使用紧凑提示面板并保留唯一确认动作", () => {
    const onClose = vi.fn();
    render(<AlertDialog title="操作未完成" message="企业应用服务暂时不可用，请稍后再试。" onClose={onClose}/>);
    expect(screen.getByRole("alertdialog").querySelector(".alert-dialog")).toBeTruthy();
    expect(screen.getAllByRole("button")).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "知道了" }));
    expect(onClose).toHaveBeenCalledOnce();
  });
});
