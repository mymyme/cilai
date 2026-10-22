import { describe, expect, it } from "vitest";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import {
  activityBarExpandedWidth, activityBarWidth, activityBarWidthFor, clampAiBrowserWidth,
  conversationMinWidth, defaultAiBrowserWidth, desiredViewportForAiBrowser, fitAiBrowser,
  maxAiBrowserWidth, minAiBrowserWidth, minChatStageWidth, minViewportForAiBrowser,
} from "./ai-layout";

/*
 * 这组用例守的是同一件事：对话页和浏览器抢同一条横向空间时，被挤破的不能是对话页。
 * 之前浏览器列写了一个 320 的保底，窗口一窄就把对话页压到它的下限以下，面板直接盖在
 * 聊天内容上——员工看到的是"输入框被切掉一截"，根本看不出是浏览器干的。
 */

const rail = 304;
const conversationMin = conversationMinWidth({ railWidth: rail, railOpen: true });

describe("对话页的最小宽度", () => {
  it("侧栏展开时是侧栏加聊天区下限", () => {
    expect(conversationMin).toBe(rail + minChatStageWidth);
  });

  it("侧栏收起后只剩聊天区下限，腾出来的地方可以给浏览器", () => {
    expect(conversationMinWidth({ railWidth: rail, railOpen: false })).toBe(minChatStageWidth);
  });

  it("侧栏被拖宽，对话页的下限跟着涨", () => {
    expect(conversationMinWidth({ railWidth: 520, railOpen: true })).toBe(520 + minChatStageWidth);
  });
});

describe("浏览器让宽", () => {
  it("空间够时给员工拖出来的宽度", () => {
    const fit = fitAiBrowser({ activityBarWidth, viewportWidth: 1600, conversationMinWidth: conversationMin, requestedWidth: 500 });
    expect(fit).toEqual({ fits: true, width: 500 });
  });

  it("空间不够员工要的宽度时，浏览器退让，对话页拿满它的下限", () => {
    const viewportWidth = activityBarWidth + conversationMin + 400;
    const fit = fitAiBrowser({ activityBarWidth, viewportWidth, conversationMinWidth: conversationMin, requestedWidth: 640 });
    expect(fit).toEqual({ fits: true, width: 400 });
    // 退让之后，剩给对话页的正好是它的下限，一像素都没少
    expect(viewportWidth - activityBarWidth - fit.width).toBe(conversationMin);
  });

  it("剩余空间低于浏览器下限就不展开，而不是留一条挤扁的空列", () => {
    const viewportWidth = activityBarWidth + conversationMin + minAiBrowserWidth - 1;
    expect(fitAiBrowser({ activityBarWidth, viewportWidth, conversationMinWidth: conversationMin, requestedWidth: 420 }))
      .toEqual({ fits: false, width: 0 });
  });

  it("正好够到下限就展开", () => {
    const viewportWidth = minViewportForAiBrowser(conversationMin, activityBarWidth);
    expect(fitAiBrowser({ activityBarWidth, viewportWidth, conversationMinWidth: conversationMin, requestedWidth: 420 }))
      .toEqual({ fits: true, width: minAiBrowserWidth });
  });

  it("窗口再怎么宽，浏览器也不超过上限——不能反过来把对话挤成一条缝", () => {
    expect(fitAiBrowser({ activityBarWidth, viewportWidth: 4000, conversationMinWidth: conversationMin, requestedWidth: 9999 }))
      .toEqual({ fits: true, width: maxAiBrowserWidth });
  });

  it("任何宽度下，对话页都不会低于自己的下限", () => {
    for (let viewportWidth = 600; viewportWidth <= 2400; viewportWidth += 7) {
      const fit = fitAiBrowser({ activityBarWidth, viewportWidth, conversationMinWidth: conversationMin, requestedWidth: 640 });
      if (!fit.fits) continue;
      expect(viewportWidth - activityBarWidth - fit.width).toBeGreaterThanOrEqual(conversationMin);
    }
  });

  it("窗口拉宽之后又放得下了：同一个判定自己会翻回来，不需要员工再点一次", () => {
    const narrow = minViewportForAiBrowser(conversationMin, activityBarWidth) - 1;
    expect(fitAiBrowser({ activityBarWidth, viewportWidth: narrow, conversationMinWidth: conversationMin, requestedWidth: 420 }).fits)
      .toBe(false);
    expect(fitAiBrowser({ activityBarWidth, viewportWidth: narrow + 1, conversationMinWidth: conversationMin, requestedWidth: 420 }).fits)
      .toBe(true);
  });

  it("侧栏收起后，原本放不下的窗口宽度变得放得下", () => {
    const viewportWidth = minViewportForAiBrowser(conversationMin, activityBarWidth) - 100;
    const collapsed = conversationMinWidth({ railWidth: rail, railOpen: false });
    expect(fitAiBrowser({ activityBarWidth, viewportWidth, conversationMinWidth: conversationMin, requestedWidth: 420 }).fits).toBe(false);
    expect(fitAiBrowser({ activityBarWidth, viewportWidth, conversationMinWidth: collapsed, requestedWidth: 420 }).fits).toBe(true);
  });
});

describe("拖动宽度的取值", () => {
  it("夹在上下限之间", () => {
    expect(clampAiBrowserWidth(10)).toBe(minAiBrowserWidth);
    expect(clampAiBrowserWidth(5000)).toBe(maxAiBrowserWidth);
    expect(clampAiBrowserWidth(453.6)).toBe(454);
  });

  it("默认宽度本身在合法范围内", () => {
    expect(clampAiBrowserWidth(defaultAiBrowserWidth)).toBe(defaultAiBrowserWidth);
  });
});


/*
 * 这一组用的是写死的真实数字，不复用 ai-layout 自己的常量。旧用例全都写成
 * `viewportWidth = activityBarWidth + conversationMin + 400` 这种形式，公式和期望值引用
 * 同一个错值，自洽但永远测不出偏差——活动栏少算 148 像素就是这么漏过去的。
 */
describe("主导航展开时，活动栏占 196 而不是 48", () => {
  it("两档宽度按主导航状态取值", () => {
    expect(activityBarWidthFor(false)).toBe(48);
    expect(activityBarWidthFor(true)).toBe(196);
    expect(activityBarWidth).toBe(48);
    expect(activityBarExpandedWidth).toBe(196);
  });

  it("展开主导航后，浏览器能占的上限少 148 像素", () => {
    // 要得足够多，两边才都撞到空间上限；要 607 的话收起那档先被记忆宽度夹住，看不出差额
    const collapsed = fitAiBrowser({
      activityBarWidth: 48, viewportWidth: 1436, conversationMinWidth: 724, requestedWidth: 720 });
    const expanded = fitAiBrowser({
      activityBarWidth: 196, viewportWidth: 1436, conversationMinWidth: 724, requestedWidth: 720 });
    expect(collapsed).toEqual({ fits: true, width: 664 });
    expect(expanded).toEqual({ fits: true, width: 516 });
    expect(collapsed.width - expanded.width).toBe(196 - 48);
  });

  it("截图里那一幕：记忆宽度 607 在展开主导航时被压到 516", () => {
    const fit = fitAiBrowser({
      activityBarWidth: 196, viewportWidth: 1436, conversationMinWidth: 724, requestedWidth: 607 });
    // 修复前这里给的是 607，多出来的 91 像素正是对话区被切掉的那一截
    expect(fit).toEqual({ fits: true, width: 516 });
  });

  it("展开主导航时对话页仍拿满下限，一像素都不少", () => {
    // 1436 / 侧栏 304 / 浏览器记忆宽度 607：修复前对话页只剩 633，被切掉 91 像素
    const fit = fitAiBrowser({
      activityBarWidth: 196, viewportWidth: 1436, conversationMinWidth: 724, requestedWidth: 607 });
    expect(1436 - 196 - fit.width).toBe(724);
  });

  it("窗口下限提示随主导航状态变化", () => {
    expect(minViewportForAiBrowser(724, 48)).toBe(1132);
    expect(minViewportForAiBrowser(724, 196)).toBe(1280);
  });
});

describe("窗口该拉到多宽", () => {
  it("拉到能放下员工记住的宽度，而不是浏览器下限", () => {
    expect(desiredViewportForAiBrowser({
      activityBarWidth: 196, conversationMinWidth: 724, requestedWidth: 607 })).toBe(1527);
  });

  it("记住的宽度超出上下限时先夹住再算", () => {
    expect(desiredViewportForAiBrowser({
      activityBarWidth: 48, conversationMinWidth: 724, requestedWidth: 10 }))
      .toBe(48 + 724 + minAiBrowserWidth);
    expect(desiredViewportForAiBrowser({
      activityBarWidth: 48, conversationMinWidth: 724, requestedWidth: 9999 }))
      .toBe(48 + 724 + maxAiBrowserWidth);
  });

  it("拉到这个宽度之后，浏览器正好拿到记住的宽度，对话页不被切", () => {
    const target = desiredViewportForAiBrowser({
      activityBarWidth: 196, conversationMinWidth: 724, requestedWidth: 607 });
    const fit = fitAiBrowser({
      activityBarWidth: 196, viewportWidth: target, conversationMinWidth: 724, requestedWidth: 607 });
    expect(fit).toEqual({ fits: true, width: 607 });
    expect(target - 196 - fit.width).toBe(724);
  });
});

describe("常量与样式表没有各说各话", () => {
  it("聊天区下限与 .ai-workbench 网格里写的是同一个数", () => {
    // 一处改了另一处没改，表现是"算出来放得下、实际却溢出"，而且不报任何错
    const sourceDir = [join(process.cwd(), "src"), join(process.cwd(), "apps", "desktop-ui", "src")]
      .find((candidate) => existsSync(join(candidate, "styles.css")));
    if (sourceDir === undefined) throw new Error("找不到 apps/desktop-ui/src");
    const css = readFileSync(join(sourceDir, "styles.css"), "utf8");
    const rule = css.match(/\.ai-workbench \{[^}]*grid-template-columns:[^;]*minmax\((\d+)px/)?.[1];
    expect(rule).toBe(String(minChatStageWidth));
  });

  /*
   * 活动栏的两档宽度此前只有样式表里有，让宽公式那边写了个常量 48 就当它永远是收起的。
   * 两处各说各话的表现是对话区被切掉一截却不报任何错，所以这里逐条钉住。
   */
  it("活动栏两档宽度与外壳网格第一列是同一个数", () => {
    const sourceDir = [join(process.cwd(), "src"), join(process.cwd(), "apps", "desktop-ui", "src")]
      .find((candidate) => existsSync(join(candidate, "styles.css")));
    if (sourceDir === undefined) throw new Error("找不到 apps/desktop-ui/src");
    const css = readFileSync(join(sourceDir, "styles.css"), "utf8");
    const firstColumn = (selector: string): string | undefined => {
      // 选择器锚在行首：.shell.navigation-expanded 是 .hide-explorer.shell.navigation-expanded 的子串
      const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      return css.match(new RegExp(`^${escaped} \\{[^}]*grid-template-columns: (\\d+)px`, "m"))?.[1];
    };
    for (const selector of [".shell", ".my-work-shell", ".hide-explorer.shell"]) {
      expect(`${selector}=${firstColumn(selector)}`).toBe(`${selector}=${activityBarWidth}`);
      const expanded = `${selector}.navigation-expanded`;
      expect(`${expanded}=${firstColumn(expanded)}`).toBe(`${expanded}=${activityBarExpandedWidth}`);
    }
  });
});
