import { describe, expect, it } from "vitest";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

/**
 * 这三条检查针对的是同一类问题：样式写了，但从来没生效过，界面上也不会报错。
 *
 * 已经出现三次——两个从未被引入的样式文件、确认弹窗里两个拼错的变量名（备注框的
 * 焦点圈因此一直画不出来）、以及 .secondary-button 这个压根没有对应规则的类名
 * （预览重试按钮因此渲染成一行纯文字）。人眼复核显然拦不住，交给测试。
 */

// vitest 可能从工作区根目录起，也可能从 apps/desktop-ui 起，两种 cwd 都认。
const sourceDir = [join(process.cwd(), "src"), join(process.cwd(), "apps", "desktop-ui", "src")]
  .find((candidate) => existsSync(join(candidate, "styles.css")));
if (sourceDir === undefined) throw new Error("找不到 apps/desktop-ui/src");

const read = (name: string) => readFileSync(join(sourceDir, name), "utf8");
// 文件系统在 Windows 返回反斜杠，源码 import 始终使用正斜杠。
const entries = readdirSync(sourceDir, { recursive: true })
  .filter((entry): entry is string => typeof entry === "string")
  .map((entry) => entry.replaceAll("\\", "/"));

const styleSheets = entries.filter((name) => name.endsWith(".css"));
/** 测试自身不算源码，否则这份文件里的示例字符串会被当成真实用法。 */
const sourceFiles = entries.filter((name) => /\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name));

/** 注释里可能出现属性名和类名，参与匹配会误报，先整段剥掉。 */
const stripCssComments = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, " ");

const allCss = stripCssComments(styleSheets.map(read).join("\n"));
const allSource = sourceFiles.map(read).join("\n");

/** 只取属性位置上的自定义属性声明，避免把 var() 引用也当成定义。 */
function definedCustomProperties(css: string): Set<string> {
  return new Set([...css.matchAll(/(?:^|[{;])\s*(--[a-z0-9-]+)\s*:/gi)].map((match) => match[1]));
}

function referencedCustomProperties(css: string): Set<string> {
  return new Set([...css.matchAll(/var\(\s*(--[a-z0-9-]+)/gi)].map((match) => match[1]));
}

/** 逐条选择器（分组规则按逗号拆开），用来发现"整条规则还活着、但其中某一段已经没人用了"。 */
function individualSelectors(css: string): string[] {
  const selectorsOnly = css.replace(/\{[^}]*\}/g, "{}");
  return [...selectorsOnly.matchAll(/(^|\})([^{}]+)\{/g)]
    .flatMap((match) => match[2].split(","))
    .map((part) => part.trim())
    .filter((part) => part.length > 0 && !part.startsWith("@"));
}

/** 选择器里出现的类名。规则体、伪类、属性选择器都不参与。 */
function classNamesInSelectors(css: string): Set<string> {
  const selectorsOnly = css.replace(/\{[^}]*\}/g, "{}");
  return new Set([...selectorsOnly.matchAll(/\.(-?[a-z][a-z0-9-]*)/gi)].map((match) => match[1]));
}

/** JSX 里 className 字面量中的类名。 */
function classNamesInSource(source: string): Set<string> {
  const found = new Set<string>();
  for (const match of source.matchAll(/className=(?:"([^"]*)"|\{`([^`]*)`\}|\{"([^"]*)"\})/g)) {
    // `${...}` 里是表达式不是类名，整段剥掉，否则变量名会被当成类名报上来。
    const literal = (match[1] ?? match[2] ?? match[3] ?? "").replace(/\$\{[^}]*\}/g, " ");
    for (const token of literal.split(/\s+/)) {
      if (/^-?[a-z][a-z0-9-]*$/i.test(token)) found.add(token);
    }
  }
  return found;
}

describe("样式表自检", () => {
  /*
   * 滚动条的外观必须由样式表定下来，不能留给操作系统。
   *
   * 留给系统的后果不是"长得朴素"，而是同一个界面时有时无地冒出一条不属于产品的粗灰条：
   * macOS 的"显示滚动条"默认按当前在用触控板还是鼠标自动切换，切换发生在运行中，已经
   * 渲染出来的容器不会全部跟着更新，于是出现的位置每次都不一样，重启又消失。
   */
  it("滚动条外观由样式表定义，两种主题各有一档滑块颜色", () => {
    const css = read("styles.css");
    expect(css).toMatch(/:root \{[^}]*scrollbar-color: var\(--scrollbar-thumb\) transparent/);
    // scrollbar-width 不可继承，只写 :root 的话页面内的滚动容器读到的仍然是 auto
    expect(stripCssComments(css)).toMatch(/\*\s*\{\s*scrollbar-width: thin;\s*\}/);
    expect(css).toMatch(/:root \{[^}]*--scrollbar-thumb:/);
    expect(css).toMatch(/:root\[data-theme="dark"\] \{[^}]*--scrollbar-thumb:/);
  });

  /*
   * ::-webkit-scrollbar 只允许用来整条隐藏。一旦用它定义外观（宽度、滑块、轨道），渲染
   * 引擎就从悬浮滚动条改走常驻滚动条：所有滚动区永久少掉一条的宽度，并且始终挂着一根条。
   * 要改粗细和颜色一律用标准属性，那两个不会破坏悬浮行为。两种写法都实测过。
   */
  it("自定义滚动条只用标准属性，不用会强制常驻的引擎前缀写法", () => {
    for (const sheet of styleSheets) {
      for (const match of stripCssComments(read(sheet)).matchAll(/::-webkit-scrollbar[^{]*\{([^}]*)\}/g)) {
        expect(`${sheet}: ${match[0].trim()}`).toMatch(/display:\s*none/);
      }
    }
  });

  it("每个样式文件都被源码引入，否则整份样式静默失效", () => {
    const orphans = styleSheets.filter((name) => !allSource.includes(`./${name}`) && !allSource.includes(`./${name.split("/").at(-1)}`)).sort();
    expect(orphans).toEqual([]);
  });

  it("var() 引用的自定义属性都有定义，否则整条声明会被丢弃", () => {
    const defined = definedCustomProperties(allCss);
    const undefinedRefs = [...referencedCustomProperties(allCss)].filter((name) => !defined.has(name)).sort();
    expect(undefinedRefs).toEqual([]);
  });

  it("JSX 里用到的类名都有对应规则，否则元素渲染出来没有外观", () => {
    const styled = classNamesInSelectors(allCss);
    const unstyled = [...classNamesInSource(allSource)].filter((name) => !styled.has(name)).sort();
    expect(unstyled).toEqual([]);
  });

  it("业务界面不再使用操作系统原生下拉组件", () => {
    const businessSource = sourceFiles.filter((name) => name !== "select.tsx").map(read).join("\n");
    expect(businessSource).not.toMatch(/<select(?:\s|>)/);
  });

  it("复合控件的单边框契约覆盖输入框和应用内选择器", () => {
    expect(allCss).toContain('[data-focus-frame="wrapper"] > :is(input, textarea, select, .ch-select)');
    expect(allCss).toContain('[data-focus-frame="wrapper"] > .ch-select[aria-expanded="true"]');
  });

  it("界面字体栈覆盖 Windows 系统界面字体和等宽字体", () => {
    const css = stripCssComments(read("styles.css"));
    expect(css).toContain('"Segoe UI Variable Text", "Segoe UI", "Microsoft YaHei UI", sans-serif');
    expect(css).toContain('--font-mono: ui-monospace, "Cascadia Mono", Consolas');
    expect(stripCssComments(read("ai-workbench.css"))).toContain("font-family: var(--font-mono)");
  });

  it("标题栏主导航开关保持 32 像素热区，22 像素图标无偏移居中", () => {
    const css = stripCssComments(read("styles.css"));
    const buttonRule = css.match(/\.title-strip \.activity-nav-toggle\s*\{([^}]*)\}/)?.[1] ?? "";
    const iconRule = css.match(/\.title-strip \.activity-nav-toggle svg\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(buttonRule).toContain("width: 32px");
    expect(buttonRule).toContain("height: 32px");
    expect(buttonRule).not.toMatch(/(?:^|;)\s*(?:top|transform)\s*:/);
    expect(iconRule).toContain("width: 22px");
    expect(iconRule).toContain("height: 22px");
  });

  it("主要页面的搜索框只通过公共组件创建", () => {
    const pages = ["applications.tsx", "automations.tsx", "my-work.tsx", "project-portfolio.tsx", "global-search.tsx"];
    for (const page of pages) {
      const source = read(page);
      expect(source).toContain("<SearchField");
      expect(source).not.toMatch(/function SearchIcon\b/);
    }
    expect(read("search-field.tsx")).toContain('data-focus-frame="wrapper"');
    expect(read("automations.tsx")).not.toContain("automations-layout");
  });

  it("公共搜索框清除内层输入外观且页面样式不重新绘制边框", () => {
    const searchCss = stripCssComments(read("search-field.css"));
    const inputRule = searchCss.match(/\.search-field\[data-focus-frame="wrapper"\]\s*>\s*input\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(inputRule).toContain("border: 0");
    expect(inputRule).toContain("border-radius: 0");
    expect(inputRule).toContain("box-shadow: none");
    expect(inputRule).toContain("background: transparent");

    const automationCss = stripCssComments(read("automations.css"));
    const automationInputRule = automationCss.match(/\.automation-search\s+input\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(automationInputRule).not.toMatch(/(?:^|;)\s*(?:border|border-radius|box-shadow|background(?:-color)?)\s*:/);
  });

  it("应用目录搜索框提供完整容器布局并约束搜索图标尺寸", () => {
    const css = stripCssComments(read("applications.css"));
    const catalogRule = css.match(/\.applications-catalog\s*\{([^}]*)\}/)?.[1] ?? "";
    const heroRule = css.match(/\.applications-hero\s*\{([^}]*)\}/)?.[1] ?? "";
    const headingRule = css.match(/\.applications-hero h2\s*\{([^}]*)\}/)?.[1] ?? "";
    const wrapperRule = css.match(/\.applications-toolbar \.search-field\s*\{([^}]*)\}/)?.[1] ?? "";
    const iconRule = css.match(/\.applications-toolbar \.search-field > svg\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(catalogRule).toContain("padding: 28px");
    expect(heroRule).toContain("margin: 0 auto 16px");
    expect(headingRule).toContain("font-size: var(--text-2xl)");
    expect(wrapperRule).toContain("display: flex");
    expect(wrapperRule).toContain("border: 1px solid var(--border-input)");
    expect(wrapperRule).toContain("background: var(--bg-surface)");
    expect(iconRule).toContain("width: 18px");
    expect(iconRule).toContain("height: 18px");
    expect(iconRule).toContain("fill: none");
    expect(iconRule).toContain("stroke: var(--text-quiet)");
  });

  it("应用目录图标保持紧凑比例", () => {
    const css = stripCssComments(read("applications.css"));
    const markRule = css.match(/(?:^|\n)\.application-mark\s*\{([^}]*)\}/)?.[1] ?? "";
    const businessIconRule = css.match(/\.application-mark \.application-business-icon\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(markRule).toContain("width: 64px");
    expect(markRule).toContain("height: 64px");
    expect(businessIconRule).toContain("width: 44px");
    expect(businessIconRule).toContain("height: 44px");
  });

  /*
   * 图标形状和配色必须分开：形状写在 tsx 里只引用色槽，具体色值留在 CSS 的色板里，
   * 浅深两套各定义一次。之前环保图标把绿色直接写死在 path 的 fill 上，底座也硬编码成
   * 深色，主题切过去图标纹丝不动。
   */
  it("应用图标只用色槽取色，两种主题各有一套色板", () => {
    const icons = read("application-icons.tsx");
    const hardcoded = icons.match(/(?:fill|stroke)="(#[0-9a-fA-F]{3,8}|rgba?\([^)]*\))"/g) ?? [];
    expect(hardcoded).toEqual([]);

    const css = stripCssComments(read("applications.css"));
    const tones = [...css.matchAll(/\.application-mark\[data-tone="([a-z]+)"\]/g)].map((match) => match[1]);
    const darkTones = [...css.matchAll(/:root\[data-theme="dark"\] \.application-mark\[data-tone="([a-z]+)"\]/g)]
      .map((match) => match[1]);
    expect(tones.length).toBeGreaterThan(0);
    // 每个色调在浅色里定义过，就必须在深色里也定义过，否则该色调切到深色会沿用浅色值。
    expect([...new Set(darkTones)].sort()).toEqual([...new Set(tones)].sort());
    for (const slot of ["--app-icon-base", "--app-icon-soft", "--app-icon-deep", "--app-icon-paper"]) {
      expect(icons).toContain(`var(${slot})`);
    }
  });

  it("对话输入区的富菜单只通过公共组件创建", () => {
    const workbench = read("ai-workbench.tsx");
    expect(workbench).toMatch(/<RichMenu\b/);
    expect(workbench).not.toContain('role="menu"');
    expect(workbench).not.toMatch(/ai-starter-(?:model|permission|project)-menu/);
  });

  it("模型选择器按完整名称自然宽度排列、不填满输入区，挤不下时收缩并省略", () => {
    /*
     * 不填满输入区靠的是 flex-grow 为 0，与能不能收缩无关。这里原先钉的是整条简写
     * `0 0 auto`，把 flex-shrink 也一并锁成 0——模型名长到一定程度（如
     * Qwen3.6-35B-A3B-NVFP4），对话页窄到底时工具条就顶破输入框。触发器里的文字本来
     * 就带省略号，但 max-width 相对父级计算，父级不收缩就永远等于内容宽度，省略号
     * 一次也不会出现。改为 `0 1 auto`：仍然不增长，只是挤不下时让位。
     */
    const css = stripCssComments(read("ai-workbench.css"));
    const picker = css.match(/\.ai-composer-actions \.ai-starter-model-picker\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(picker).toMatch(/flex:\s*0 1 auto/);
    expect(picker).toContain("overflow: visible");
    // 省略号的三件套缺一不可，少任何一个都会变成硬裁切或撑开
    const span = css.match(/\.ch-rich-menu-trigger > span\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(span).toContain("overflow: hidden");
    expect(span).toContain("text-overflow: ellipsis");
    expect(span).toContain("white-space: nowrap");
    expect(css).toContain("width: max-content; grid-template-columns: minmax(0, 1fr) 11px");
    expect(css).not.toContain(".ai-composer-actions .ai-starter-model-trigger { width: 100%");
  });

  it("输入框的焦点态只改变唯一外框颜色，投影与静止态相同", () => {
    const css = stripCssComments(read("styles.css"));
    const rest = css.match(/\.ai-composer\s*\{([^}]*)\}/)?.[1] ?? "";
    const focusRule = css.match(/\.ai-composer:focus-within\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(focusRule).toContain("border-color: var(--border-strong)");
    expect(rest).toContain("box-shadow: var(--shadow-md)");
    expect(focusRule).toContain("box-shadow: var(--shadow-md)");
    expect(focusRule).not.toContain("inset");
  });

  it("对话行悬停时不会用通用按钮底色盖住持续选中态", () => {
    const css = stripCssComments(read("styles.css"));
    expect(css).toContain(".ai-conversation-item.selected:hover");
    expect(css).toContain(".ai-conversation-rail .ai-conversation-open:hover");
  });

  it("回到底部按钮按下时保持与静止态相同的位置变换", () => {
    const css = stripCssComments(read("styles.css"));
    const buttonRule = css.match(/\.ai-workbench button\.ai-return-to-latest\s*\{([^}]*)\}/)?.[1] ?? "";
    const activeRule = css.match(/\.ai-workbench button\.ai-return-to-latest:active\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(buttonRule).toContain("width: 36px");
    expect(buttonRule).toContain("height: 36px");
    expect(buttonRule).toContain("border-radius: 50%");
    expect(activeRule).toContain("transform: translateX(50%)");
    expect(activeRule).not.toContain("translateY");
  });

  it("流式回复与完成回复使用相同的 Markdown 空白排版", () => {
    const css = stripCssComments(read("styles.css"));
    const streamingRule = css.match(/\.ai-streaming-output\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(streamingRule).not.toContain("white-space: pre-wrap");
    expect(streamingRule).not.toContain("white-space: break-spaces");
  });

  it("执行过程用时入口不显示按钮卡片且只在收起时保留分隔线", () => {
    const css = stripCssComments(read("ai-workbench.css"));
    const toggleRule = css.match(/\.ai-workbench \.ai-execution-toggle\s*\{([^}]*)\}/)?.[1] ?? "";
    const collapsedRule = css.match(/\.ai-workbench \.ai-execution-toggle\[aria-expanded="false"\]\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(toggleRule).toContain("border: 0");
    expect(toggleRule).toContain("border-radius: 0");
    expect(toggleRule).toContain("background: transparent");
    expect(toggleRule).toContain("display: flex");
    expect(toggleRule).toContain("justify-content: flex-start");
    expect(toggleRule).not.toContain("minmax(0, 1fr)");
    expect(collapsedRule).toContain("border-bottom: 1px solid var(--border-subtle)");
  });

  it("生成图片在对话中使用紧凑预览且全屏尺寸保持独立", () => {
    const css = stripCssComments(read("ai-workbench.css"));
    const galleryRule = css.match(/\.ai-run-image-gallery\s*\{([^}]*)\}/)?.[1] ?? "";
    const mainRule = css.match(/\.ai-run-image-main\s*\{([^}]*)\}/)?.[1] ?? "";
    const imageRule = css.match(/\.ai-run-image-main > img\s*\{([^}]*)\}/)?.[1] ?? "";
    const lightboxImageRule = stripCssComments(read("artifact-preview.css")).match(/\.ai-artifact-lightbox figure img\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(galleryRule).toContain("width: min(100%, 680px)");
    expect(mainRule).toContain("min-height: 220px");
    expect(mainRule).toContain("max-height: 500px");
    expect(imageRule).toContain("max-width: 100%");
    expect(imageRule).toContain("max-height: 500px");
    expect(lightboxImageRule).toContain("max-height: calc(100dvh - 148px)");
  });

  it("消息复制和编辑操作只显示图标而不显示按钮容器", () => {
    const css = stripCssComments(read("ai-workbench.css"));
    const buttonRule = css.match(/\.ai-workbench \.ai-message-actions button\s*\{([^}]*)\}/)?.[1] ?? "";
    const hoverRule = css.match(/\.ai-workbench \.ai-message-actions button:hover:not\(:disabled\)\s*\{([^}]*)\}/)?.[1] ?? "";
    const activeRule = css.match(/\.ai-workbench \.ai-message-actions button:active\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(buttonRule).toContain("border: 0");
    expect(buttonRule).toContain("border-radius: 0");
    expect(buttonRule).toContain("background: transparent");
    expect(buttonRule).toContain("box-shadow: none");
    expect(hoverRule).toContain("background: transparent");
    expect(activeRule).toContain("transform: none");
  });

  it("对话项目只在交互时高亮，更多按钮不会形成独立色块", () => {
    const css = stripCssComments(read("styles.css"));
    expect(css).not.toContain(".ai-conversation-group-header.selected");
    expect(css).toContain(".ai-conversation-group-header:hover");
    expect(css).not.toContain("--conversation-group-selected-bg");
    const toggleHoverRule = css.match(/\.ai-conversation-rail \.ai-conversation-group-toggle:hover[^\{]*\{([^}]*)\}/)?.[1] ?? "";
    expect(toggleHoverRule).toContain("background: transparent");
    expect(toggleHoverRule).toContain("transform: none");
    const moreButtonRule = css.match(/\.ai-conversation-rail \.ai-conversation-group-actions button:hover[^\{]*\{([^}]*)\}/)?.[1] ?? "";
    expect(moreButtonRule).toContain("background: transparent");
    expect(moreButtonRule).toContain("transform: none");
  });

});

describe("样式表里没有已经没人用的选择器", () => {
  it("分组规则里的每一段都还指向源码里存在的类名", () => {
    // 上一轮清理只删了"整条规则的所有选择器都死了"的那些，像
    // `.ai-workbench svg, .outcome-ai-entry svg` 这种活的和死的写在一起的漏了下来：
    // 规则还在生效，但其中一段永远匹配不到任何东西。
    // 这一条比"JSX 类名都有对应规则"宽松：类名可能是在模板串里拼出来的
    // （`shell${personalPage ? " my-work-shell" : ""}`），只要名字在源码里出现过就算在用。
    // 宁可漏报也不能误报——一条会误报的检查，最后一定是被人改成永远通过。
    const mentioned = (name: string) => allSource.includes(name);
    const orphaned = individualSelectors(allCss)
      .filter((selector) => {
        const names = [...selector.matchAll(/\.(-?[a-z][a-z0-9-]*)/gi)].map((match) => match[1]);
        return names.length > 0 && names.every((name) => !mentioned(name));
      })
      .sort();
    expect([...new Set(orphaned)]).toEqual([]);
  });
});

describe("对话页里的图标按钮没有被通用按钮样式盖出框来", () => {
  it("每条 .ai-browser-toggle 规则都带 .ai-workbench 作用域", () => {
    /*
     * 对话页有一条 `.ai-workbench button` 给页内所有按钮加边框和底色，权重比单独的类名
     * 高一档。工具栏那颗地球图标当初没加作用域，界面上就凭空多出一个方框，而样式本身
     * 写的是 border: 0 —— 看代码完全看不出问题，只有在真界面上才显形。
     */
    const generic = stripCssComments(allCss).match(/\.ai-workbench button\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(generic).toMatch(/border\s*:/);
    // 只管按钮本身：它的子元素（如提示点）是 span，不会被 `.ai-workbench button` 命中
    const unscoped = individualSelectors(stripCssComments(read("ai-browser.css")))
      .filter((selector) => /\.ai-browser-toggle(?![\w-])/.test(selector) && !selector.includes(".ai-workbench"));
    expect(unscoped).toEqual([]);
  });
});

describe("起始引导与进行中的对话共用同一套结构", () => {
  it("起始引导只是消息区里的一种内容，没有另一套外层与输入框", () => {
    /*
     * 员工发出第一句话、或重新进入继续上次的对话时，界面不该整块换成另一套页面——
     * 输入框变窄、消息挪位、工具按钮冒出来，看上去就是页面切了一次。起始引导与对话记录
     * 放在同一个消息区里，输入框和工具栏始终是同一份。
     */
    const css = stripCssComments(read("ai-workbench.css"));
    expect(css).not.toMatch(/\.ai-starter\s*\{/);
    expect(css).not.toContain(".ai-starter-composer");
    expect(css).not.toContain(".ai-starter-pending-messages");
    const landing = css.match(/\.ai-message-list-landing\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(landing).toContain("display: flex");
    expect(landing).toContain("flex-direction: column");
    const content = css.match(/\.ai-starter-content\s*\{([^}]*)\}/)?.[1] ?? "";
    // 引导在消息区剩下的空间里居中；滚动交给消息区，引导自己不再另开滚动条
    expect(content).toContain("justify-content: center");
    expect(content).toMatch(/flex:\s*1 0 auto/);
    expect(content).not.toContain("overflow");
    // 与消息、输入框同宽，三者左右对齐
    expect(content).toContain("width: min(780px, 100%)");
  });
});

describe("对话页里的按钮类不被通用按钮样式盖出框来", () => {
  it("每条针对按钮的类规则都带作用域", () => {
    /*
     * `.ai-workbench button` 给页内所有按钮加边框、圆角和底色，权重比单独的类名高一档。
     * 这个坑在同一轮里踩了两次：工具栏的地球图标凭空多出方框，导航条的 2 像素细线被
     * 套成胶囊——两次源码里都明明写着 border: 0，只有在真界面上才看得出来。
     * 所以这条检查不再只盯某一个类名，而是覆盖对话页里所有自带按钮样式的组件。
     */
    const generic = stripCssComments(allCss).match(/\.ai-workbench button\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(generic).toMatch(/border\s*:/);
    const guarded = ["ai-browser.css", "conversation-navigator.css"];
    const unscoped = guarded.flatMap((file) => individualSelectors(stripCssComments(read(file)))
      .filter((selector) => /(?:^|\s)\.[\w-]*(?:toggle|tick)(?![\w-])/.test(selector)
        && !/\.ai-workbench|\.conversation-navigator(?![\w-])/.test(selector)));
    expect(unscoped).toEqual([]);
  });
});

/*
 * 这两组几何关系一旦写漏，界面上看到的是"位置不对"而不是"样式没生效"，
 * 排查时很容易怪到别处。都是已经犯过的：
 * 导航条只写了 top: 50% 忘了回拉半个自身高度，整条下坠一半；
 * 滑轨用实体边框，内容盒比边框盒窄两像素，刻度和滑块的百分比基准就此分家。
 */
describe("几何关系自检", () => {
  it("对话导航条按固定条数限高，回拉半个自身高度，且溢出时能滚到最早几条", () => {
    const nav = stripCssComments(read("conversation-navigator.css"))
      .match(/\.conversation-navigator\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(nav).toMatch(/height:\s*calc\(var\(--navigator-tick-height\)\s*\*\s*var\(--navigator-visible-ticks\)\)/);
    expect(nav).toMatch(/overflow-y:\s*auto/);
    // top 的百分比定的是上边缘，少了这一句整条会往下掉半个容器高
    expect(nav).toMatch(/top:\s*50%/);
    expect(nav).toMatch(/transform:\s*translateY\(-50%\)/);
    // 普通的 center 会把溢出均分到两端，起始方向那几条永远滚不到
    expect(nav).toMatch(/justify-content:\s*safe center/);
    expect(nav).toMatch(/scrollbar-width:\s*none/);
  });

  it("思考深度滑轨与滑块等高，滑块圆心与刻度点取同一个位置", () => {
    const css = stripCssComments(read("ai-workbench.css"));
    const track = css.match(/\.ai-reasoning-effort-track\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(track).toMatch(/height:\s*var\(--reasoning-slider-size\)/);
    // 描边走 inset 阴影：实体边框会让内容盒比边框盒窄，百分比基准随之分家
    expect(track).toMatch(/border:\s*0/);
    expect(track).toMatch(/box-shadow:\s*inset/);

    const thumb = css.match(/\.ai-reasoning-effort-thumb\s*\{([^}]*)\}/)?.[1] ?? "";
    // 圆心直接取刻度点那个值，拖到底才贴得住右端
    expect(thumb).toMatch(/left:\s*var\(--selected-position\)/);
    expect(thumb).toMatch(/transform:\s*translate\(-50%,\s*-50%\)/);
    // 拖动时手指底下不该有一圈描边，立体感交给投影
    expect(thumb).toMatch(/border:\s*0/);
    expect(thumb).not.toMatch(/inset 0 0 0 1px/);
    // 原生那颗只留交互，视觉全部交给自绘的圆
    expect(css).toMatch(/::-webkit-slider-thumb\s*\{[^}]*background:\s*transparent/);
  });
});

it("拖动思考深度滑块时不在整条轨道外面画焦点圈", () => {
  const css = stripCssComments(read("ai-workbench.css"));
  // 通用的 input:focus 画的是 :focus，只挡 :focus-visible 的话框会换个颜色继续出现
  const focused = css.match(/\.ai-reasoning-effort-track > input:focus, \.ai-reasoning-effort-track > input:focus-visible\s*\{([^}]*)\}/)?.[1] ?? "";
  expect(focused).toMatch(/box-shadow:\s*none/);
  expect(focused).toMatch(/outline:\s*none/);
  // 键盘可达性仍要留痕迹，缩到滑块周围；按住不放时连这一圈也收起
  expect(css).toMatch(/:has\(> input:focus-visible:not\(:active\)\) \.ai-reasoning-effort-thumb/);
});

it("模型列表与思考深度两步之间的尺寸变化有过渡，且首次定位不参与", () => {
  const css = stripCssComments(read("ai-workbench.css"));
  const menu = css.match(/\.ai-model-picker-menu\[data-settled="true"\]\s*\{([^}]*)\}/)?.[1] ?? "";
  // 组件把测得的内容高度写进 max-height，元素高度是 auto，过渡 height 不会有效果
  expect(menu).toMatch(/max-height\s+\d+ms/);
  expect(menu).toMatch(/width\s+\d+ms/);
  // 菜单首帧摆在屏幕外等待测量，不隔开这一帧就会从屏幕外滑进来
  expect(read("rich-menu.tsx")).toContain('data-settled=');
  expect(read("rich-menu.tsx")).toContain("requestAnimationFrame(() => setSettled(true))");
  // 动效要尊重系统的减弱动态偏好
  expect(css).toMatch(/@media \(prefers-reduced-motion: reduce\)[^}]*\{[\s\S]{0,400}?ai-picker-step-in|animation:\s*none/);
});
