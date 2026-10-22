import { describe, expect, it } from "vitest";
import { displayModelId } from "./model-label";

describe("下线模型的显示名", () => {
  it("网关生成的随机编号显示为原模型", () => {
    expect(displayModelId("3ab9c430-28d0-42b4-9dba-b676d0a77dc1")).toBe("原模型");
  });

  it("可读的模型编号原样保留", () => {
    expect(displayModelId("agnes-3.0-flash")).toBe("agnes-3.0-flash");
    expect(displayModelId("removed-model")).toBe("removed-model");
  });
});
