import { describe, expect, it } from "vitest";
import { describeServerError, serverErrorReason } from "./server-error";

it("宿主及界面的已知导入错误也按精确原因显示", () => {
  expect(describeServerError(new Error("personal_package_import_response_invalid"))).toBe("导入记录无法核对，操作已停止。请刷新记录。");
  expect(describeServerError("personal_package_import_target_invalid")).toBe("导入目标已失效，请重新选择。");
});

/** 真实形状：服务端错误经 JSON-RPC 包一层后到达前端。 */
const protocolError = (reason: string) =>
  new Error(`App Server returned protocol error: {"code":-32010,"data":{"reason":"${reason}","retryable":false},"message":"Workspace creation failed"}`);

describe("服务端错误的人话版本", () => {
  it("从协议错误里挑出 reason", () => {
    expect(serverErrorReason(protocolError("workspace_root_unavailable"))).toBe("workspace_root_unavailable");
  });

  it("所选目录不可读时给出原因，不显示协议原文", () => {
    const text = describeServerError(protocolError("workspace_root_unavailable"));
    expect(text).toContain("读不到这个文件夹");
    expect(text).not.toContain("-32010");
  });

  it("应用目录失败不向员工显示协议错误", () => {
    const text = describeServerError(protocolError("application_catalog_request_failed"));
    expect(text).toBe("企业应用服务暂时不可用，请稍后再试。");
    expect(text).not.toContain("-32010");
    expect(text).not.toContain("application_catalog_request_failed");
  });

  it("认不出来的原因保持原样：藏起一个没预料到的错误比显示得难看更糟", () => {
    const unknown = protocolError("some_reason_we_have_not_seen");
    expect(describeServerError(unknown)).toBe(String(unknown));
  });

  it("完全不是协议错误的也原样返回", () => {
    expect(describeServerError(new Error("网络断了"))).toContain("网络断了");
    expect(serverErrorReason(new Error("网络断了"))).toBeNull();
  });
});
