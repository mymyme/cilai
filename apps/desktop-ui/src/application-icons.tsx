/*
 * 企业目录没给图标时，工作台自己画一个。
 *
 * 以前这里只看业务分类，同一分类下的所有应用拿到的是同一张图——环保分类的
 * 「RTO运行分析」「第三方报告」「环保档案」在货架上长得一模一样，员工只能靠底下那行
 * 字区分。分类是给筛选用的，粒度本来就不适合当图标依据。
 *
 * 改成按应用自己的名称和关键词推断它在做什么：分析、审查、归档是三件不同的事，
 * 哪怕都挂在环保底下。推断不出来才退回分类，分类也认不出才用通用图标。
 *
 * 配色不写死在 path 上，只用色调变量。同一套形状在浅色和深色主题下取不同的值，
 * 由 applications.css 里的色板负责，图标文件不关心当前是什么主题。
 */

import type { ReactElement } from "react";

export type ApplicationIconTone =
  | "heatmap"
  | "eco"
  | "analytics"
  | "compliance"
  | "archive"
  | "emission"
  | "finance"
  | "education"
  | "manufacturing"
  | "people"
  | "collaboration"
  | "neutral";

type IconKind = ApplicationIconTone;

/** 命中即定，顺序即优先级：先认具体做什么，再认属于哪个领域。 */
const SEMANTIC_RULES: ReadonlyArray<readonly [IconKind, readonly string[]]> = [
  ["heatmap", ["空气质量热力图", "heatmap"]],
  ["compliance", ["合规", "审查", "审核", "稽核", "检查", "认证", "许可", "资质", "风控", "audit", "compliance"]],
  ["analytics", ["分析", "运行", "监测", "监控", "趋势", "统计", "报表", "看板", "指标", "诊断", "analytics", "monitor", "dashboard"]],
  ["archive", ["档案", "归档", "资料", "文档", "台账", "记录", "存证", "archive", "record"]],
  ["emission", ["排放", "废气", "废水", "污染", "碳", "能耗", "环境", "emission", "carbon"]],
  ["finance", ["财务", "金融", "账", "结算", "成本", "预算", "发票", "资金", "finance", "billing"]],
  ["education", ["教育", "培训", "课程", "学习", "考试", "题库", "education", "training"]],
  ["manufacturing", ["生产", "制造", "工单", "排产", "设备", "产线", "工艺", "检修", "manufacturing", "production"]],
  ["people", ["人力", "人事", "员工", "考勤", "招聘", "绩效", "薪酬", "hr", "people"]],
  ["collaboration", ["协作", "协同", "项目", "任务", "审批", "流程", "工作流", "collaboration", "project"]],
];

const CATEGORY_KIND: Record<string, IconKind> = {
  环保: "eco",
  金融: "finance",
  经营管理: "finance",
  教育: "education",
  生产制造: "manufacturing",
  人力资源: "people",
  项目协作: "collaboration",
  协同办公: "collaboration",
  企业管理: "collaboration",
};

/**
 * 推断一个应用该用哪张图标。
 *
 * 名称和关键词优先：名称最能说明这个应用在做什么，关键词补充名称里没写全的业务词。
 * 应用清单不带关键词，名称又只写业务对象时（如「第三方报告」），再看简介。简介写得
 * 比名称宽，常顺带提到别的业务，所以只在前两处认不出时才用。都认不出才退回分类。
 */
export function applicationIconKind(input: {
  name: string;
  keywords: readonly string[];
  description?: string;
  categoryLabel: string;
}): IconKind {
  const matched =
    matchSemanticKind([input.name, ...input.keywords]) ??
    matchSemanticKind([input.description ?? ""]);
  return matched ?? CATEGORY_KIND[input.categoryLabel] ?? "neutral";
}

function matchSemanticKind(texts: readonly string[]): IconKind | undefined {
  const haystack = texts.join(" ").toLocaleLowerCase();
  return SEMANTIC_RULES.find(([, terms]) => terms.some((term) => haystack.includes(term)))?.[0];
}

/**
 * 图标只用四个色槽，不出现具体色值：
 * base 主体、soft 亮面、deep 暗面、paper 主体上的留白。
 */
const BASE = "var(--app-icon-base)";
const SOFT = "var(--app-icon-soft)";
const DEEP = "var(--app-icon-deep)";
const PAPER = "var(--app-icon-paper)";

export function ApplicationBusinessIcon(props: { kind: IconKind }) {
  return (
    <svg className="application-business-icon" viewBox="0 0 64 64" aria-hidden="true">
      {ICONS[props.kind]}
    </svg>
  );
}

const ICONS: Record<IconKind, ReactElement> = {
  heatmap: (
    <>
      <rect x="4" y="4" width="56" height="56" rx="16" fill={DEEP} />
      <path d="M12 35c0-13 12-25 25-22 13 2 20 14 14 27-5 12-21 16-32 8-5-3-7-8-7-13Z" fill={BASE} />
      <path d="M21 33c1-9 10-15 18-12 9 3 11 11 6 19-5 7-16 8-21 2-3-3-4-6-3-9Z" stroke={SOFT} strokeWidth="2.5" fill="none" />
      <path d="M29 32c2-4 7-6 11-3 4 4 1 10-4 10-5 1-9-3-7-7Z" fill="var(--app-icon-hot)" />
      <circle cx="35" cy="33" r="3" fill={PAPER} />
    </>
  ),
  // 档案盒：盒体加抽出的卡片，表示归档与检索。
  archive: (
    <>
      <path d="M16 12h24c5 0 9 4 9 9v20H16c-5 0-9-4-9-9V21c0-5 4-9 9-9Z" fill={SOFT} />
      <rect x="7" y="22" width="50" height="35" rx="11" fill={BASE} />
      <path d="M7 35c12-7 31-9 50-4v15c0 6-5 11-11 11H18C12 57 7 52 7 46V35Z" fill={DEEP} />
      <rect x="19" y="30" width="26" height="18" rx="6" fill={PAPER} />
      <path d="M25 35h14v3H25zM28 41h8v3h-8z" fill={DEEP} />
    </>
  ),
  // 柱状图加趋势线：运行数据的分析与走势。
  analytics: (
    <>
      <rect x="7" y="14" width="50" height="43" rx="12" fill={BASE} />
      <path d="M7 34c13-6 32-8 50-3v15c0 6-5 11-11 11H18C12 57 7 52 7 46V34Z" fill={DEEP} />
      <rect x="17" y="34" width="7" height="13" rx="3" fill={PAPER} />
      <rect x="28" y="28" width="7" height="19" rx="3" fill={PAPER} />
      <rect x="39" y="22" width="7" height="25" rx="3" fill={PAPER} />
      <path d="M17 26l9-6 8 4 12-10" stroke={SOFT} strokeWidth="3.4" strokeLinecap="round" strokeLinejoin="round" fill="none" />
    </>
  ),
  // 盾牌加勾：通过审查、符合规则。
  compliance: (
    <>
      <path d="M32 6l20 7v18c0 13-8 22-20 27C20 53 12 44 12 31V13l20-7Z" fill={SOFT} />
      <path d="M32 11l15 5v15c0 10-6 17-15 21-9-4-15-11-15-21V16l15-5Z" fill={BASE} />
      <path d="M32 32v20c-9-4-15-11-15-21V16l15-5v21Z" fill={DEEP} />
      <path d="M24 31.5l6 6 11-12" stroke={PAPER} strokeWidth="4.6" strokeLinecap="round" strokeLinejoin="round" fill="none" />
    </>
  ),
  // 叶片加水滴：环保领域的通用标识。
  eco: (
    <>
      <rect x="7" y="7" width="50" height="50" rx="15" fill={BASE} />
      <path d="M7 32c13-6 32-8 50-3v13c0 8-7 15-15 15H22c-8 0-15-7-15-15V32Z" fill={DEEP} />
      <path d="M44 16c2 14-6 25-19 26-2-14 6-25 19-26Z" fill={SOFT} />
      <path d="M21 45c5-11 12-18 22-22" stroke={PAPER} strokeWidth="3" strokeLinecap="round" fill="none" />
    </>
  ),
  // 烟囱加读数波形：排放与能耗监测。
  emission: (
    <>
      <rect x="7" y="7" width="50" height="50" rx="15" fill={BASE} />
      <path d="M7 34c13-6 32-8 50-3v11c0 8-7 15-15 15H22c-8 0-15-7-15-15V34Z" fill={DEEP} />
      <path d="M24 24h9l3 24H21l3-24Z" fill={PAPER} />
      <path d="M38 20c5-2 8 1 7 5s-6 5-8 2" stroke={SOFT} strokeWidth="3.2" strokeLinecap="round" fill="none" />
      <path d="M14 44l5-5 5 4 6-7" stroke={SOFT} strokeWidth="3.2" strokeLinecap="round" strokeLinejoin="round" fill="none" />
    </>
  ),
  // 币面加上行柱：资金与经营指标。
  finance: (
    <>
      <rect x="7" y="7" width="50" height="50" rx="15" fill={BASE} />
      <path d="M7 33c13-6 32-8 50-3v12c0 8-7 15-15 15H22c-8 0-15-7-15-15V33Z" fill={DEEP} />
      <circle cx="26" cy="26" r="11" fill={SOFT} />
      <path d="M26 19v14M22 23h8M22 29h8" stroke={PAPER} strokeWidth="2.8" strokeLinecap="round" fill="none" />
      <rect x="38" y="30" width="6" height="17" rx="3" fill={PAPER} />
      <rect x="47" y="24" width="6" height="23" rx="3" fill={PAPER} />
    </>
  ),
  // 翻开的书：课程与培训。
  education: (
    <>
      <rect x="7" y="7" width="50" height="50" rx="15" fill={BASE} />
      <path d="M7 34c13-6 32-8 50-3v11c0 8-7 15-15 15H22c-8 0-15-7-15-15V34Z" fill={DEEP} />
      <path d="M13 18c8-2 14-1 19 4v25c-5-5-11-6-19-4V18Z" fill={PAPER} />
      <path d="M51 18c-8-2-14-1-19 4v25c5-5 11-6 19-4V18Z" fill={SOFT} />
    </>
  ),
  // 齿轮加厂房：生产与设备。
  manufacturing: (
    <>
      <rect x="7" y="7" width="50" height="50" rx="15" fill={BASE} />
      <path d="M7 34c13-6 32-8 50-3v11c0 8-7 15-15 15H22c-8 0-15-7-15-15V34Z" fill={DEEP} />
      <path d="M31 13l4 1 2-4 5 3-1 4 4 2-2 6-4-1-3 3 1 4-6 2-2-4h-4l-2 4-5-3 1-4-4-2 2-6 4 1 3-3-1-4 6-2 2 4Z" fill={SOFT} />
      <circle cx="31" cy="25" r="5" fill={PAPER} />
      <path d="M17 46v-8l8 4 8-4v8" stroke={PAPER} strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" fill="none" />
    </>
  ),
  // 双人：人力与组织。
  people: (
    <>
      <rect x="7" y="7" width="50" height="50" rx="15" fill={BASE} />
      <path d="M7 34c13-6 32-8 50-3v11c0 8-7 15-15 15H22c-8 0-15-7-15-15V34Z" fill={DEEP} />
      <circle cx="41" cy="25" r="7" fill={SOFT} />
      <path d="M30 48c0-7 5-12 11-12s11 5 11 12H30Z" fill={SOFT} />
      <circle cx="25" cy="23" r="8" fill={PAPER} />
      <path d="M12 49c0-8 6-13 13-13s13 5 13 13H12Z" fill={PAPER} />
    </>
  ),
  // 看板列：任务流转与协同。
  collaboration: (
    <>
      <rect x="7" y="7" width="50" height="50" rx="15" fill={BASE} />
      <path d="M7 34c13-6 32-8 50-3v11c0 8-7 15-15 15H22c-8 0-15-7-15-15V34Z" fill={DEEP} />
      <rect x="15" y="16" width="13" height="26" rx="5" fill={PAPER} />
      <rect x="32" y="16" width="13" height="17" rx="5" fill={SOFT} />
      <path d="M38 40v8M34 44l4 4 4-4" stroke={PAPER} strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" fill="none" />
    </>
  ),
  // 四格：认不出业务时的通用应用标识。
  neutral: (
    <>
      <rect x="7" y="7" width="50" height="50" rx="15" fill={BASE} />
      <path d="M7 34c13-6 32-8 50-3v11c0 8-7 15-15 15H22c-8 0-15-7-15-15V34Z" fill={DEEP} />
      <rect x="17" y="17" width="12" height="12" rx="4" fill={PAPER} />
      <rect x="35" y="17" width="12" height="12" rx="4" fill={SOFT} />
      <rect x="17" y="35" width="12" height="12" rx="4" fill={SOFT} />
      <rect x="35" y="35" width="12" height="12" rx="4" fill={PAPER} />
    </>
  ),
};
