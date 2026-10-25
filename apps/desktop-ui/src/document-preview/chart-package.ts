import type { Element } from "@xmldom/xmldom";
import type { PreviewChart, PreviewChartLayer, PreviewChartSeries, PreviewChartType } from "./model";
import { child, children, color, elements, num, text, xml, type Package } from "./package-xml";

const chartTypes: Record<string, PreviewChartType> = { barChart: "bar", lineChart: "line", areaChart: "area", pieChart: "pie", doughnutChart: "doughnut", scatterChart: "scatter", radarChart: "radar" };
const colors = ["#4472c4", "#ed7d31", "#a5a5a5", "#ffc000", "#5b9bd5", "#70ad47"];
const MAX_POINTS = 10000;
const MAX_TOTAL_POINTS = 20000;
function flag(parent: Element | undefined, name: string): boolean { return ["1", "true"].includes(child(parent, name)?.getAttribute("val") ?? ""); }
function cachedValues(source: Element | undefined): string[] {
  if (!source) return [];
  const caches = elements(source, "strCache").concat(elements(source, "numCache"), elements(source, "strLit"), elements(source, "numLit"));
  const cache = caches[0] ?? child(source, "lvl");
  if (!cache) {
    const levels = elements(source, "lvl");
    if (levels.length) return cachedLevel(levels[levels.length - 1]);
    throw new Error("图表数据未包含可读取的缓存。");
  }
  return cachedLevel(cache);
}
function cachedLevel(cache: Element): string[] {
  const points = children(cache, "pt");
  if (points.length > MAX_POINTS) throw new Error("preview_package_limits");
  const count = Math.max(num(child(cache, "ptCount"), "val"), ...points.map(point => num(point, "idx") + 1), 0);
  if (!Number.isInteger(count) || count > MAX_POINTS) throw new Error("图表数据超过预览上限。");
  const values = Array<string>(count).fill("");
  for (const point of points) {
    const index = num(point, "idx");
    if (!Number.isInteger(index) || index < 0 || index >= count) throw new Error("图表数据索引无效。");
    values[index] = child(point, "v")?.textContent ?? "";
  }
  return values;
}
function numbers(source: Element | undefined): (number | null)[] {
  return cachedValues(source).map(value => {
    if (value === "") return null;
    const result = Number(value);
    if (!Number.isFinite(result)) throw new Error("图表包含无效数值。");
    return result;
  });
}
export function readChart(pkg: Package, part: string, theme: Map<string, string>): PreviewChart {
  const doc = xml(pkg, part); const chart = elements(doc, "chart")[0]; const plot = child(chart, "plotArea");
  if (!chart || !plot) throw new Error("图表结构无效。");
  const layers: PreviewChartLayer[] = []; let categories: string[] = [];
  for (const node of children(plot).filter(node => node.localName?.endsWith("Chart"))) {
    const type = chartTypes[node.localName ?? ""];
    if (!type) throw new Error(`不支持此图表类型：${node.localName}。`);
    const groupingValue = child(node, "grouping")?.getAttribute("val") ?? "standard";
    if (!["standard", "clustered", "stacked", "percentStacked"].includes(groupingValue)) throw new Error("图表分组方式不受支持。");
    const series: PreviewChartSeries[] = children(node, "ser").map((ser, index) => {
      const tx = child(ser, "tx");
      const name = tx ? child(tx, "v")?.textContent ?? cachedValues(tx)[0] ?? `系列 ${index + 1}` : `系列 ${index + 1}`;
      const labels = child(ser, "cat") ? cachedValues(child(ser, "cat")) : [];
      if (labels.length > categories.length) categories = labels;
      const values = numbers(child(ser, type === "scatter" ? "yVal" : "val"));
      if (!values.length) throw new Error("图表系列没有可绘制的数据。");
      const themeColor = theme.get(`accent${index % 6 + 1}`);
      return { name, values, ...(type === "scatter" ? { xValues: numbers(child(ser, "xVal")) } : {}), color: color(child(child(ser, "spPr"), "solidFill"), theme) ?? (themeColor && /^[\da-f]{6}$/i.test(themeColor) ? `#${themeColor}` : colors[index % colors.length]) };
    });
    if (!series.length || series.length > 100) throw new Error("图表系列数量无效。");
    layers.push({ type, series, horizontal: child(node, "barDir")?.getAttribute("val") === "bar", grouping: groupingValue === "stacked" || groupingValue === "percentStacked" ? groupingValue : "standard", holeSize: num(child(node, "holeSize"), "val", 50) });
  }
  if (!layers.length) throw new Error("图表没有支持的绘图区域。");
  if (layers.flatMap(layer => layer.series).reduce((sum, series) => sum + series.values.length, 0) > MAX_TOTAL_POINTS) throw new Error("preview_package_limits");
  if (children(plot, "valAx").length > (layers.every(layer => layer.type === "scatter") ? 2 : 1)) throw new Error("此图表的多数值轴尚不支持。");
  if (layers.length > 1 && layers.some(layer => ["pie", "doughnut", "radar", "scatter"].includes(layer.type) || layer.horizontal)) throw new Error("此组合图表尚不支持。");
  for (const layer of layers) {
    if (["pie", "doughnut"].includes(layer.type) && layer.series.some(series => series.values.some(value => value !== null && value < 0))) throw new Error("含负值的饼形或环形图尚不支持。");
    if (["pie", "doughnut"].includes(layer.type) && layer.series.length !== 1) throw new Error("此多环图表尚不支持。");
    if (layer.type === "radar" && layer.series.some(series => series.values.some(value => value === null || value < 0))) throw new Error("此雷达图包含尚不支持的空值或负值。");
    if (layer.grouping === "percentStacked" && layer.series.some(series => series.values.some(value => value !== null && value < 0))) throw new Error("含负值的百分比堆叠图尚不支持。");
    if (layer.type === "scatter" && layer.series.some(series => series.xValues?.length !== series.values.length)) throw new Error("散点图坐标数量不一致。");
  }
  const labels = elements(chart, "dLbls")[0];
  const axes = children(plot, "valAx"); const scatter = layers[0].type === "scatter";
  const xAxis = scatter ? axes.find(axis => ["b", "t"].includes(child(axis, "axPos")?.getAttribute("val") ?? "")) : undefined;
  const yAxis = scatter ? axes.find(axis => ["l", "r"].includes(child(axis, "axPos")?.getAttribute("val") ?? "")) : axes[0];
  if (scatter && axes.length && (!xAxis || !yAxis || axes.length !== 2)) throw new Error("散点图坐标轴关系无效。");
  const readAxis = (axis: Element | undefined, values: number[]) => {
    const scaling = child(axis, "scaling");
    const value = (name: string) => { const item = child(scaling, name); return item ? num(item, "val") : undefined; };
    const min = value("min"); const max = value("max"); const logBase = value("logBase");
    if (min !== undefined && max !== undefined && min >= max) throw new Error("图表数值轴范围无效。");
    if (logBase !== undefined && (logBase < 2 || logBase > 1000 || values.some(value => value <= 0) || min !== undefined && min <= 0 || max !== undefined && max <= 0)) throw new Error("图表对数轴范围或数据无效。");
    return { min, max, logBase, reversed: child(scaling, "orientation")?.getAttribute("val") === "maxMin" };
  };
  const y = readAxis(yAxis, layers.flatMap(layer => layer.series.flatMap(series => series.values.filter((value): value is number => value !== null))));
  const x = readAxis(xAxis, layers.flatMap(layer => layer.series.flatMap(series => series.xValues?.filter((value): value is number => value !== null) ?? [])));
  if (y.logBase !== undefined && layers.some(layer => layer.grouping !== "standard")) throw new Error("对数堆叠图尚不支持。");
  const blankDisplay = child(chart, "dispBlanksAs")?.getAttribute("val") ?? "gap";
  if (!["gap", "zero", "span"].includes(blankDisplay)) throw new Error("图表空值处理方式无效。");
  return { blankDisplay: blankDisplay as "gap" | "zero" | "span", title: child(chart, "title") ? text(child(chart, "title")!) : "", categories, layers, legend: Boolean(child(chart, "legend")), showValues: flag(labels, "showVal"), showPercent: flag(labels, "showPercent"), min: y.min, max: y.max, logBase: y.logBase, reversed: y.reversed, xMin: x.min, xMax: x.max, xLogBase: x.logBase, xReversed: x.reversed };
}
