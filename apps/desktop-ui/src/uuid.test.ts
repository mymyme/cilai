import { describe, expect, it } from "vitest";
import { randomUuid } from "./uuid";

describe("randomUuid", () => {
  it("生成符合 UUID v4 规范的随机标识", () => {
    expect(randomUuid()).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  });
});
