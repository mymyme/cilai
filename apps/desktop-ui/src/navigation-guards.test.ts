import { describe, expect, it } from "vitest";
import { renderHook } from "@testing-library/react";
import { useNavigationGuards } from "./navigation-guards";

const guards = () => renderHook(() => useNavigationGuards()).result.current;

describe("启动恢复只跑一次", () => {
  it("第一次认领成功", () => {
    expect(guards().claimRestore()).toBe(true);
  });

  it("认领过就不会再跑：恢复是一次性的，跑第二次会把用户当前的位置顶掉", () => {
    const g = guards();
    g.claimRestore();
    expect(g.claimRestore()).toBe(false);
  });

  it("用户先动过就不再恢复：本地服务要几秒才就绪，恢复晚一步抢回页面最恼人", () => {
    const g = guards();
    g.userNavigated();
    expect(g.claimRestore()).toBe(false);
  });

  it("离开项目同样作废未跑的恢复", () => {
    const g = guards();
    g.projectExited();
    expect(g.claimRestore()).toBe(false);
  });
});

describe("在途的项目解析", () => {
  it("没人打断时结果有效", () => {
    const g = guards();
    const stillCurrent = g.beginProjectResolve();
    expect(stillCurrent()).toBe(true);
  });

  it("解析期间离开项目，晚到的结果作废：否则会把用户重新拽回刚离开的项目", () => {
    const g = guards();
    const stillCurrent = g.beginProjectResolve();
    g.projectExited();
    expect(stillCurrent()).toBe(false);
  });

  it("普通导航不作废解析：点侧栏去别的页面时项目上下文本来就该留着", () => {
    const g = guards();
    const stillCurrent = g.beginProjectResolve();
    g.userNavigated();
    expect(stillCurrent()).toBe(true);
  });

  it("两次解析各自独立，后一次不会让前一次误判为有效", () => {
    const g = guards();
    const first = g.beginProjectResolve();
    g.projectExited();
    const second = g.beginProjectResolve();
    expect(first()).toBe(false);
    expect(second()).toBe(true);
  });
});

describe("恢复期间用户是否插手", () => {
  it("用户没动时页面可以由恢复决定", () => {
    const g = guards();
    const moved = g.watchUserNavigation();
    expect(moved()).toBe(false);
  });

  it("用户动过则页面听用户的，上下文仍照常恢复", () => {
    const g = guards();
    const moved = g.watchUserNavigation();
    g.userNavigated();
    expect(moved()).toBe(true);
  });

  it("离开项目不计作用户翻页", () => {
    const g = guards();
    const moved = g.watchUserNavigation();
    g.projectExited();
    expect(moved()).toBe(false);
  });
});
