import type { ReactNode } from "react";
import type { PreviewChart, PreviewChartSeries } from "./model";

const label = (value: number) => new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 2, notation: Math.abs(value) >= 100000 ? "compact" : "standard" }).format(value);
function sector(cx: number, cy: number, outer: number, inner: number, start: number, end: number): string {
  const point = (radius: number, angle: number) => `${cx + radius * Math.cos(angle)},${cy + radius * Math.sin(angle)}`;
  const large = end - start > Math.PI ? 1 : 0;
  return inner > 0 ? `M${point(outer, start)} A${outer},${outer} 0 ${large} 1 ${point(outer, end)} L${point(inner, end)} A${inner},${inner} 0 ${large} 0 ${point(inner, start)} Z` : `M${cx},${cy} L${point(outer, start)} A${outer},${outer} 0 ${large} 1 ${point(outer, end)} Z`;
}
const palette = ["#4472c4", "#ed7d31", "#a5a5a5", "#ffc000", "#5b9bd5", "#70ad47"];
export function ChartView({ chart, width, height }: { chart: PreviewChart; width: number; height: number }) {
  const plot = { x: 42, y: chart.title ? 32 : 12, width: Math.max(20, width - 60), height: Math.max(20, height - (chart.title ? 68 : 48) - (chart.legend ? 24 : 0)) };
  const layer = chart.layers[0]; const radial = ["pie", "doughnut", "radar"].includes(layer.type);
  const nodes: ReactNode[] = [];
  const allSeries = chart.layers.flatMap(layer => layer.series);
  if (radial && layer.type !== "radar") {
    const series = layer.series[0]; const total = series.values.reduce<number>((sum, value) => sum + Math.abs(value ?? 0), 0);
    const radius = Math.max(1, Math.min(plot.width, plot.height) / 2); const cx = plot.x + plot.width / 2; const cy = plot.y + plot.height / 2;
    let angle = -Math.PI / 2;
    series.values.forEach((value, index) => {
      if (!value || !total) return;
      const end = angle + Math.abs(value) / total * (Math.PI * 2 - 1e-8);
      const color = palette[index % palette.length];
      nodes.push(<path key={`slice-${index}`} d={sector(cx, cy, radius, layer.type === "doughnut" ? radius * Math.min(90, Math.max(10, layer.holeSize)) / 100 : 0, angle, end)} fill={color} stroke="white"><title>{chart.categories[index] ?? index + 1}: {value}</title></path>);
      if (chart.showValues || chart.showPercent) nodes.push(<text key={`slice-label-${index}`} x={cx + radius * .7 * Math.cos((angle + end) / 2)} y={cy + radius * .7 * Math.sin((angle + end) / 2)} textAnchor="middle" dominantBaseline="central" fontSize="11" fill="#111">{chart.showPercent ? `${label(Math.abs(value) / total * 100)}%` : label(value)}</text>);
      angle = end;
    });
  } else if (layer.type === "radar") {
    const count = Math.max(chart.categories.length, ...layer.series.map(series => series.values.length)); const radius = Math.min(plot.width, plot.height) / 2; const cx = plot.x + plot.width / 2; const cy = plot.y + plot.height / 2;
    const maximum = Math.max(1, ...layer.series.flatMap(series => series.values.filter((value): value is number => value !== null)));
    const point = (index: number, value: number) => `${cx + Math.cos(index / count * Math.PI * 2 - Math.PI / 2) * radius * value},${cy + Math.sin(index / count * Math.PI * 2 - Math.PI / 2) * radius * value}`;
    for (let index = 0; index < count; index++) {
      const [x, y] = point(index, 1).split(",").map(Number);
      nodes.push(<line key={`radar-axis-${index}`} x1={cx} y1={cy} x2={x} y2={y} stroke="#d5d8dc"/>);
      nodes.push(<text key={`radar-label-${index}`} x={x} y={y - 4} fontSize="10" textAnchor="middle">{chart.categories[index] ?? index + 1}</text>);
    }
    layer.series.forEach((series, index) => nodes.push(<polygon key={`radar-${index}`} points={series.values.map((value, i) => point(i, (value ?? 0) / maximum)).join(" ")} fill={`${series.color}33`} stroke={series.color}><title>{series.name}</title></polygon>));
  } else {
    const horizontal = layer.type === "bar" && layer.horizontal;
    const scatter = layer.type === "scatter";
    const allX = allSeries.flatMap(series => series.xValues?.filter((value): value is number => value !== null) ?? []);
    const xMin = chart.xMin ?? (allX.length ? Math.min(...allX) : 0); const xMax = chart.xMax ?? (allX.length ? Math.max(...allX) : 1);
    const xTransform = (value: number) => chart.xLogBase ? Math.log(value) / Math.log(chart.xLogBase) : value;
    const xFraction = (value: number) => { const fraction = (xTransform(value) - xTransform(xMin)) / (xTransform(xMax) - xTransform(xMin) || 1); return chart.xReversed ? 1 - fraction : fraction; };
    const percentAxis = chart.layers.every(layer => layer.grouping === "percentStacked");
    const axisLabel = (value: number) => percentAxis ? `${label(value * 100)}%` : label(value);
    const categoryCount = Math.max(chart.categories.length, ...allSeries.map(series => series.values.length));
    const values = chart.layers.flatMap(layer => layer.grouping === "standard" ? layer.series.flatMap(series => series.values.filter((value): value is number => value !== null)) : Array.from({ length: categoryCount }, (_, index) => layer.grouping === "percentStacked" ? (layer.series.some(series => (series.values[index] ?? 0) > 0) ? 1 : 0) : layer.series.reduce((sum, series) => sum + Math.max(0, series.values[index] ?? 0), 0)).concat(Array.from({ length: categoryCount }, (_, index) => layer.grouping === "percentStacked" ? (layer.series.some(series => (series.values[index] ?? 0) < 0) ? -1 : 0) : layer.series.reduce((sum, series) => sum + Math.min(0, series.values[index] ?? 0), 0))));
    const minimum = chart.min ?? Math.min(0, ...values); const dataMaximum = chart.max ?? Math.max(0, ...values); const maximum = dataMaximum === minimum ? minimum + 1 : dataMaximum;
    const transform = (value: number) => chart.logBase ? Math.log(Math.max(Number.MIN_VALUE, value)) / Math.log(chart.logBase) : value;
    const min = transform(chart.logBase && minimum <= 0 ? Math.min(...values.filter(value => value > 0)) : minimum); const max = transform(maximum);
    const ratio = (value: number) => { const fraction = (transform(value) - min) / (max - min || 1); return chart.reversed ? 1 - fraction : fraction; };
    const valuePosition = (value: number) => horizontal ? plot.x + ratio(value) * plot.width : plot.y + (1 - ratio(value)) * plot.height;
    const zero = valuePosition(chart.logBase ? Math.pow(chart.logBase, min) : 0);
    for (let tick = 0; tick <= 4; tick++) {
      const fraction = tick / 4; const value = chart.logBase ? Math.pow(chart.logBase, min + (max - min) * fraction) : minimum + (maximum - minimum) * fraction;
      const position = valuePosition(value);
      nodes.push(horizontal ? <g key={`tick-${tick}`}><line x1={position} y1={plot.y} x2={position} y2={plot.y + plot.height} stroke="#e4e6e8"/><text x={position} y={plot.y + plot.height + 13} textAnchor="middle" fontSize="9">{axisLabel(value)}</text></g> : <g key={`tick-${tick}`}><line x1={plot.x} y1={position} x2={plot.x + plot.width} y2={position} stroke="#e4e6e8"/><text x={plot.x - 5} y={position + 3} textAnchor="end" fontSize="9">{axisLabel(value)}</text></g>);
    }
    chart.layers.forEach((layer, layerIndex) => {
      const positive = Array<number>(categoryCount).fill(0); const negative = [...positive];
      layer.series.forEach((series, seriesIndex) => {
        const segments: [number, number, number][][] = [[]];
        const points = () => segments[segments.length - 1];
        series.values.forEach((cached, index) => {
          const raw = cached === null && chart.blankDisplay === "zero" ? 0 : cached;
          if (raw === null || layer.type === "scatter" && series.xValues?.[index] === null) {
            if (chart.blankDisplay !== "span" && points().length) segments.push([]);
            return;
          }
          const total = layer.series.reduce((sum, series) => sum + Math.abs(series.values[index] ?? 0), 0);
          const value = layer.grouping === "percentStacked" ? raw / (total || 1) : raw;
          const stacked = layer.grouping !== "standard"; const previous = stacked ? (value >= 0 ? positive[index] : negative[index]) : 0; const next = previous + value;
          if (value >= 0) positive[index] = next; else negative[index] = next;
          const category = (index + .5) / categoryCount;
          const x = layer.type === "scatter" ? plot.x + xFraction(series.xValues?.[index] ?? 0) * plot.width : plot.x + category * plot.width;
          const y = valuePosition(next);
          const tooltip = `${series.name} · ${chart.categories[index] ?? series.xValues?.[index] ?? index + 1}: ${raw}`;
          if (layer.type === "bar") {
            const slot = (horizontal ? plot.height : plot.width) / categoryCount * .76; const barSize = stacked ? slot : slot / layer.series.length;
            const start = stacked ? valuePosition(previous) : zero; const finish = valuePosition(next);
            const categoryPosition = (horizontal ? plot.y + category * plot.height : x) - slot / 2 + (stacked ? 0 : seriesIndex * barSize);
            nodes.push(<rect key={`bar-${layerIndex}-${seriesIndex}-${index}`} x={horizontal ? Math.min(start, finish) : categoryPosition} y={horizontal ? categoryPosition : Math.min(start, finish)} width={horizontal ? Math.abs(finish - start) : barSize} height={horizontal ? barSize : Math.abs(finish - start)} fill={series.color}><title>{tooltip}</title></rect>);
          } else {
            points().push([x, y, stacked ? valuePosition(previous) : zero]);
            nodes.push(<circle key={`point-${layerIndex}-${seriesIndex}-${index}`} cx={x} cy={y} r={layer.type === "scatter" ? 3 : 2} fill={series.color}><title>{tooltip}</title></circle>);
          }
          if (chart.showValues) nodes.push(<text key={`value-${layerIndex}-${seriesIndex}-${index}`} x={horizontal ? valuePosition(next) + 3 : x} y={horizontal ? plot.y + category * plot.height : y - 4} fontSize="9" textAnchor={horizontal ? "start" : "middle"}>{label(raw)}</text>);
        });
        if (layer.type === "line" || layer.type === "area") {
          segments.filter(segment => segment.length).forEach((segment, segmentIndex) => {
            const path = segment.map(([x, y], index) => `${index ? "L" : "M"}${x},${y}`).join(" ");
            const baseline = [...segment].reverse().map(([x, _y, base]) => `L${x},${base}`).join(" ");
            nodes.push(<path key={`line-${layerIndex}-${seriesIndex}-${segmentIndex}`} d={layer.type === "area" ? `${path} ${baseline} Z` : path} fill={layer.type === "area" ? `${series.color}66` : "none"} stroke={series.color} strokeWidth="2"/>);
          });
        }
      });
    });
    if (scatter) {
      for (let tick = 0; tick <= 4; tick++) nodes.push(<text key={`x-tick-${tick}`} x={plot.x + tick / 4 * plot.width} y={plot.y + plot.height + 14} textAnchor="middle" fontSize="9">{label(chart.xLogBase ? Math.pow(chart.xLogBase, xTransform(chart.xReversed ? xMax : xMin) + (xTransform(chart.xReversed ? xMin : xMax) - xTransform(chart.xReversed ? xMax : xMin)) * tick / 4) : (chart.xReversed ? xMax - (xMax - xMin) * tick / 4 : xMin + (xMax - xMin) * tick / 4))}</text>);
    } else {
      for (let index = 0; index < categoryCount; index++) nodes.push(horizontal ? <text key={`category-${index}`} x={plot.x - 4} y={plot.y + (index + .5) / categoryCount * plot.height + 3} textAnchor="end" fontSize="9">{chart.categories[index] ?? index + 1}</text> : <text key={`category-${index}`} x={plot.x + (index + .5) / categoryCount * plot.width} y={plot.y + plot.height + 14} textAnchor="middle" fontSize="9">{chart.categories[index] ?? index + 1}</text>);
    }
  }
  const legendEntries: Pick<PreviewChartSeries, "name" | "color">[] = radial && layer.type !== "radar" ? layer.series[0].values.map((_value, index) => ({ name: chart.categories[index] ?? String(index + 1), color: palette[index % palette.length] })) : allSeries;
  return <svg aria-label={chart.title || "图表"} role="img" width={width} height={height} viewBox={`0 0 ${width} ${height}`} style={{ fontFamily: "Arial, Microsoft YaHei, sans-serif", color: "#222", background: "white" }}><title>{chart.title || "图表"}</title>{chart.title ? <text x={width / 2} y="20" textAnchor="middle" fontSize="14" fontWeight="600">{chart.title}</text> : null}{nodes}{chart.legend ? legendEntries.map((entry, index) => <g key={`legend-${index}`} transform={`translate(${10 + index * (width - 20) / legendEntries.length},${height - 13})`}><rect width="8" height="8" fill={entry.color}/><text x="12" y="8" fontSize="10">{entry.name}</text></g>) : null}</svg>;
}
