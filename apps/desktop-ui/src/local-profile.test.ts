import { afterEach, expect, it, vi } from "vitest";
import { loadLocalProfile } from "./local-profile";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
afterEach(() => invoke.mockReset());
const profile = { profileId: "11111111111111111111111111111111", schemaVersion: 2, displayName: "本地资料", ready: true };
it("只接纳已就绪的新版个人资料", async () => {
  invoke.mockResolvedValue(profile); expect(await loadLocalProfile()).toEqual(profile);
});
it.each([{ ...profile, schemaVersion: 1 }, { ...profile, schemaVersion: 3 }, { ...profile, ready: false }, { ...profile, profileId: "tenant-id" }])("不将旧版、未来版或未就绪资料视为启动通过", async (value) => {
  invoke.mockResolvedValue(value); await expect(loadLocalProfile()).rejects.toThrow("本地资料格式不受支持");
});
