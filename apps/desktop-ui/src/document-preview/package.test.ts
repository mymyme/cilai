import { describe, expect, test } from "vitest";
import { strToU8, zipSync, unzipSync } from "fflate";
import { readDocumentPackage } from "./package";
function archive(parts: Record<string, string | Uint8Array>) { return new Uint8Array(zipSync(Object.fromEntries(Object.entries(parts).map(([name, text]) => [name, typeof text === "string" ? strToU8(text) : text])), { level: 0 })).buffer; }
const ns = 'xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"';
const transform = '<a:xfrm><a:off x="12700" y="25400"/><a:ext cx="1270000" cy="635000"/></a:xfrm>';
function presentation(slide: string, extra: Record<string, string | Uint8Array> = {}) {
  return archive({ 'ppt/presentation.xml': `<p:presentation ${ns}><p:sldIdLst><p:sldId id="2" r:id="rSlide"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>`, 'ppt/_rels/presentation.xml.rels': '<Relationships><Relationship Id="rSlide" Target="slides/slide2.xml"/></Relationships>', 'ppt/slides/slide2.xml': `<p:sld ${ns}><p:cSld><p:spTree>${slide}</p:spTree></p:cSld></p:sld>`, ...extra });
}
describe('Office文档受控解析', () => {
  test('按演示文稿关系读取页面并保留文字格式、形状坐标和表格', () => {
    const model = readDocumentPackage(presentation(`<p:sp><p:nvSpPr><p:cNvPr id="9"/></p:nvSpPr><p:spPr>${transform}<a:prstGeom prst="ellipse"/><a:solidFill><a:srgbClr val="12AB34"/></a:solidFill></p:spPr><p:txBody><a:p><a:r><a:rPr b="1" sz="2400"/><a:t>项目计划</a:t></a:r></a:p></p:txBody></p:sp><p:graphicFrame><p:xfrm><a:off x="0" y="0"/><a:ext cx="1270000" cy="635000"/></p:xfrm><a:graphic><a:graphicData><a:tbl><a:tr><a:tc><a:txBody><a:p><a:r><a:t>表格内容</a:t></a:r></a:p></a:txBody></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>`), 'pptx');
    expect(model.format).toBe('pptx'); if (model.format !== 'pptx') return;
    expect(model.slides[0].objects[0]).toMatchObject({ kind: 'text', shape: 'ellipse', fill: '#12AB34', geometry: { x: 12700, y: 25400, width: 1270000 }, paragraphs: [{ runs: [{ text: '项目计划', bold: true, size: 24 }] }] });
    expect(model.slides[0].objects[1]).toMatchObject({ kind: 'table', rows: [['表格内容']] });
  });
  test('图片读取受控包内资源，图表和SmartArt只形成预览占位', () => {
    const model = readDocumentPackage(presentation(`<p:pic><p:spPr>${transform}</p:spPr><p:blipFill><a:blip r:embed="rImage"/><a:srcRect l="10000"/></p:blipFill></p:pic><p:graphicFrame><p:xfrm><a:off x="4" y="5"/><a:ext cx="40" cy="50"/></p:xfrm><a:graphic><a:chart/></a:graphic></p:graphicFrame><p:graphicFrame><p:xfrm><a:off x="6" y="7"/><a:ext cx="60" cy="70"/></p:xfrm><a:graphic><a:relIds/></a:graphic></p:graphicFrame>`, { 'ppt/slides/_rels/slide2.xml.rels': '<Relationships><Relationship Id="rImage" Target="../media/photo.png"/></Relationships>', 'ppt/media/photo.png': new Uint8Array([137,80,78,71]) }), 'pptx');
    if (model.format !== 'pptx') throw new Error('wrong format');
    expect(model.slides[0].objects[0]).toMatchObject({ kind: 'image', image: 'data:image/png;base64,iVBORw==', crop: { left: .1 } });
    expect(model.slides[0].objects.slice(1)).toMatchObject([{ kind: 'placeholder', placeholder: '图表', geometry: { x: 4, y: 5, width: 40, height: 50 } }, { kind: 'placeholder', placeholder: 'SmartArt' }]);
  });
  test('Excel保留公式和缓存状态，不将缓存当作已重算结果', () => {
    const model = readDocumentPackage(archive({ 'xl/workbook.xml': '<workbook xmlns:r="urn:r"><sheets><sheet name="营收" sheetId="1" r:id="r1"/></sheets><calcPr fullCalcOnLoad="1"/></workbook>', 'xl/_rels/workbook.xml.rels': '<Relationships><Relationship Id="r1" Target="worksheets/sheet1.xml"/></Relationships>', 'xl/worksheets/sheet1.xml': '<worksheet><sheetData><row><c r="A1" t="inlineStr"><is><t>营收</t></is></c><c r="B2"><f>SUM(B3:B4)</f><v>15</v></c></row></sheetData><mergeCells><mergeCell ref="A1:C1"/></mergeCells></worksheet>' }), 'xlsx');
    expect(model).toMatchObject({ format: 'xlsx', calculationState: 'stale', sheets: [{ name: '营收', cells: [{ value: '营收' }, { value: '15', formula: 'SUM(B3:B4)' }], merges: [{ startRow: 1, startColumn: 1, endColumn: 3 }] }] });
  });
  test('XMind按真实包内容区分JSON和XML并保留层级及备注', () => {
    const json = readDocumentPackage(archive({ 'content.json': JSON.stringify([{ id: 'sheet', title: '方案', rootTopic: { id: 'root', title: '目标', notes: { plain: { content: '备注' } }, children: { attached: [{ id: 'child', title: '行动' }] } } }]) }), 'xmind');
    expect(json).toMatchObject({ format: 'xmind', sheets: [{ root: { title: '目标', notes: '备注', children: [{ title: '行动' }] } }] });
    const xml = readDocumentPackage(archive({ 'content.xml': '<xmap-content><sheet id="s"><title>XML</title><topic id="r"><title>根</title><children><topics type="attached"><topic id="c"><title>子</title></topic></topics></children></topic></sheet></xmap-content>' }), 'xmind');
    expect(xml).toMatchObject({ format: 'xmind', sheets: [{ root: { title: '根', children: [{ title: '子' }] } }] });
  });
  test('DOCX解析前删除外部关系且不修改输入包', () => {
    const bytes = archive({ 'word/document.xml': '<w:document xmlns:w="urn:w"><w:body/></w:document>', 'word/_rels/document.xml.rels': '<Relationships><Relationship Id="r1" Target="https://outside.example/image.png" TargetMode="External"/></Relationships>' });
    const original = new Uint8Array(bytes).slice(); const model = readDocumentPackage(bytes, 'docx');
    if (model.format !== 'docx') throw new Error('wrong format');
    expect(new TextDecoder().decode(unzipSync(new Uint8Array(model.bytes))['word/_rels/document.xml.rels'])).not.toContain('outside.example');
    expect(new Uint8Array(bytes)).toEqual(original);
  });
  test('拒绝路径穿越、DTD和超量解包，不展示部分结果', () => {
    expect(() => readDocumentPackage(archive({ '../content.json': '[]' }), 'xmind')).toThrow('preview_package_invalid');
    expect(() => readDocumentPackage(archive({ 'content.xml': '<!DOCTYPE x [<!ENTITY a SYSTEM "file:///etc/passwd">]><x>&a;</x>' }), 'xmind')).toThrow('preview_package_unsafe');
    expect(() => readDocumentPackage(archive({ 'content.json': '[]', 'large.bin': new Uint8Array(49 * 1024 * 1024) }), 'xmind')).toThrow('preview_file_too_large');
  });
});
