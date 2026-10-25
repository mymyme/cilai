import { render, screen } from "@testing-library/react";
import { strToU8, zipSync } from "fflate";
import { describe, expect, test } from "vitest";
import { readChart } from "./chart-package";
import { ChartView } from "./chart-view";
import { readDocumentPackage } from "./package";
import { StructuredDocumentPreview } from "./viewer";
const ns = 'xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"';
function pptx(graphic: string, parts: Record<string, string>) {
  const files = { 'ppt/presentation.xml': `<p:presentation ${ns}><p:sldIdLst><p:sldId id="1" r:id="s"/></p:sldIdLst></p:presentation>`, 'ppt/_rels/presentation.xml.rels': '<Relationships><Relationship Id="s" Target="slides/slide1.xml"/></Relationships>', 'ppt/slides/slide1.xml': `<p:sld ${ns}><p:cSld><p:spTree><p:graphicFrame><p:xfrm><a:off x="12700" y="25400"/><a:ext cx="6350000" cy="3810000"/></p:xfrm><a:graphic><a:graphicData>${graphic}</a:graphicData></a:graphic></p:graphicFrame></p:spTree></p:cSld></p:sld>`, ...parts };
  return readDocumentPackage(new Uint8Array(zipSync(Object.fromEntries(Object.entries(files).map(([name, value]) => [name, strToU8(value)])), { level: 0 })).buffer, 'pptx');
}
const nums = (values: (number | null)[]) => `<c:numLit><c:ptCount val="${values.length}"/>${values.map((value, i) => value === null ? '' : `<c:pt idx="${i}"><c:v>${value}</c:v></c:pt>`).join('')}</c:numLit>`;
function chart(type = 'barChart', series = `<c:ser><c:tx><c:v>收入</c:v></c:tx><c:cat><c:strLit><c:pt idx="0"><c:v>一月</c:v></c:pt><c:pt idx="1"><c:v>二月</c:v></c:pt></c:strLit></c:cat><c:val>${nums([10, -3])}</c:val></c:ser>`, axes = '') {
  return pptx('<c:chart r:id="c"/>', { 'ppt/slides/_rels/slide1.xml.rels': '<Relationships><Relationship Id="c" Target="../charts/chart1.xml"/></Relationships>', 'ppt/charts/chart1.xml': `<c:chartSpace ${ns}><c:chart><c:title><a:p><a:r><a:t>月度收入</a:t></a:r></a:p></c:title><c:plotArea><c:${type}>${series}</c:${type}>${axes}</c:plotArea><c:legend/></c:chart></c:chartSpace>` });
}
describe('Office图形实际解析与显示', () => {
  test('PPTX图表读取包内数值、分类及标题，并绘制真实柱形', () => {
    const model = chart();
    if (model.format !== 'pptx') throw new Error('wrong format');
    expect(model.slides[0].objects[0]).toMatchObject({ kind: 'chart', chart: { title: '月度收入', categories: ['一月', '二月'], layers: [{ type: 'bar', series: [{ name: '收入', values: [10, -3] }] }] } });
    const { container } = render(<StructuredDocumentPreview model={model}/>);
    expect(screen.getByRole('img', { name: '月度收入' })).toBeTruthy();
    expect(container.querySelectorAll('rect')).toHaveLength(3);
    expect(container.textContent).toContain('一月: 10');
    expect(container.textContent).toContain('二月: -3');
  });
  test('折线空点形成断开，散点不同系列共享数值横轴', () => {
    const line = chart('lineChart', `<c:ser><c:val>${nums([1, null, 3])}</c:val></c:ser>`);
    if (line.format !== 'pptx' || !line.slides[0].objects[0].chart) throw new Error('wrong model');
    const { container, unmount } = render(<ChartView chart={line.slides[0].objects[0].chart} width={500} height={300}/>);
    expect(container.querySelectorAll('path')).toHaveLength(2); unmount();
    const scatter = chart('scatterChart', [[0, 10], [10, 20]].map(values => `<c:ser><c:xVal>${nums(values)}</c:xVal><c:yVal>${nums([2, 4])}</c:yVal></c:ser>`).join(''));
    if (scatter.format !== 'pptx' || !scatter.slides[0].objects[0].chart) throw new Error('wrong model');
    const view = render(<ChartView chart={scatter.slides[0].objects[0].chart} width={500} height={300}/>);
    const points = [...view.container.querySelectorAll('circle')].map(circle => Number(circle.getAttribute('cx')));
    expect(points[1]).toBe(points[2]); expect(points[0]).toBeLessThan(points[1]); expect(points[2]).toBeLessThan(points[3]);
  });
  test('散点图分别使用横纵数值轴的范围与反向设置', () => {
    const model = chart('scatterChart', `<c:ser><c:xVal>${nums([10,20])}</c:xVal><c:yVal>${nums([2,4])}</c:yVal></c:ser>`, '<c:valAx><c:axPos val="b"/><c:scaling><c:min val="0"/><c:max val="100"/></c:scaling></c:valAx><c:valAx><c:axPos val="l"/><c:scaling><c:min val="0"/><c:max val="10"/></c:scaling></c:valAx>');
    if (model.format !== 'pptx' || !model.slides[0].objects[0].chart) throw new Error('wrong model');
    expect(model.slides[0].objects[0].chart).toMatchObject({ min: 0, max: 10, xMin: 0, xMax: 100 });
    const { container } = render(<ChartView chart={model.slides[0].objects[0].chart} width={500} height={300}/>);
    const circles = [...container.querySelectorAll('circle')];
    expect(circles.map(circle => Number(circle.getAttribute('cx')))).toEqual([86,130]);
    expect(Number(circles[0].getAttribute('cy'))).toBeCloseTo(198.4);
    expect(Number(circles[1].getAttribute('cy'))).toBeCloseTo(156.8);
  });
  test('正值百分比堆叠柱填满0到100%数值轴', () => {
    const model = chart('barChart', `<c:grouping val="percentStacked"/><c:ser><c:val>${nums([2])}</c:val></c:ser><c:ser><c:val>${nums([3])}</c:val></c:ser>`);
    if (model.format !== 'pptx' || !model.slides[0].objects[0].chart) throw new Error('wrong model');
    const { container } = render(<ChartView chart={model.slides[0].objects[0].chart} width={500} height={300}/>);
    expect(container.textContent).toContain('100%'); expect(container.textContent).not.toContain('-100%');
    const bars = [...container.querySelectorAll('rect')].filter(rect => rect.querySelector('title'));
    expect(bars.reduce((sum, bar) => sum + Number(bar.getAttribute('height')), 0)).toBeCloseTo(208);
  });
  test('SmartArt跟随data及drawing关系读取实际形状和文字', () => {
    const model = pptx('<dgm:relIds r:dm="d"/>', { 'ppt/slides/_rels/slide1.xml.rels': '<Relationships><Relationship Id="d" Target="../diagrams/data1.xml"/></Relationships>', 'ppt/diagrams/data1.xml': '<dgm:dataModel xmlns:dgm="urn:diagram" xmlns:dsp="urn:drawing"><dsp:dataModelExt relId="drawing"/></dgm:dataModel>', 'ppt/diagrams/_rels/data1.xml.rels': '<Relationships><Relationship Id="drawing" Type="http://schemas.microsoft.com/office/2007/relationships/diagramDrawing" Target="drawing1.xml"/></Relationships>', 'ppt/diagrams/drawing1.xml': `<dsp:drawing xmlns:dsp="urn:drawing" ${ns}><dsp:spTree><dsp:grpSpPr/><dsp:sp><dsp:nvSpPr><dsp:cNvPr id="9"/></dsp:nvSpPr><dsp:spPr><a:xfrm><a:off x="100" y="200"/><a:ext cx="1000000" cy="500000"/></a:xfrm><a:prstGeom prst="roundRect"/><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></dsp:spPr><dsp:txBody><a:p><a:r><a:t>实际流程节点</a:t></a:r></a:p></dsp:txBody></dsp:sp></dsp:spTree></dsp:drawing>` });
    if (model.format !== 'pptx') throw new Error('wrong format');
    expect(model.slides[0].objects[0]).toMatchObject({ kind: 'group', viewport: { x: 100, y: 200, width: 1000000, height: 500000 }, children: [{ kind: 'text', shape: 'roundRect', paragraphs: [{ runs: [{ text: '实际流程节点' }] }] }] });
    render(<StructuredDocumentPreview model={model}/>); expect(screen.getByText('实际流程节点')).toBeTruthy();
  });
  test('图表主题色输出有效SVG颜色，负饼图明确拒绝', () => {
    const pkg = { 'chart.xml': strToU8(`<c:chartSpace ${ns}><c:chart><c:plotArea><c:barChart><c:ser><c:val>${nums([2])}</c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>`) };
    expect(readChart(pkg, 'chart.xml', new Map([['accent1','112233']])).layers[0].series[0].color).toBe('#112233');
    const model = chart('pieChart');
    if (model.format !== 'pptx') throw new Error('wrong model');
    expect(model.slides[0].objects[0]).toMatchObject({ kind: 'error', error: '含负值的饼形或环形图尚不支持。' });
  });
  test('不支持的3D图表明确显示失败原因', () => {
    const model = chart('bar3DChart');
    if (model.format !== 'pptx') throw new Error('wrong format');
    expect(model.slides[0].objects[0]).toMatchObject({ kind: 'error', error: '不支持此图表类型：bar3DChart。' });
    render(<StructuredDocumentPreview model={model}/>); expect(screen.getByRole('alert').textContent).toContain('bar3DChart');
  });
});
