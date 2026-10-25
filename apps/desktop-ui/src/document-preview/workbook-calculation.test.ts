import { describe, expect, test } from "vitest";
import type { PreviewCell, PreviewSheet } from "./model";
import { recalculateWorkbook, translateSharedFormula } from "./workbook-calculation";
function cell(address: string, row: number, column: number, value: string, formula?: string): PreviewCell {
  return { address, row, column, value, valueType: "number", ...(formula ? { formula, cachedValue: value } : {}) };
}
function sheet(name: string, cells: PreviewCell[]): PreviewSheet { return { id: name, name, cells, rowCount: 10, columnCount: 10, merges: [] }; }
describe("Excel公式重算", () => {
  test("依赖图重算跨表、范围和定义名称，缓存不参与结果", () => {
    const book = [sheet("收入", [cell("A1", 1, 1, "2"), cell("A2", 2, 1, "3"), cell("B1", 1, 2, "999", "SUM(A1:A2)*2"), cell("C1", 1, 3, "888", "B1+'成本'!A1+SUM(Total)"), cell("D1", 1, 4, "777", "IF(C1=22,ROUND(2.345,2),0)")]), sheet("成本", [cell("A1", 1, 1, "7")])];
    expect(recalculateWorkbook(book, [{ name: "Total", value: "收入!$A$1:$A$2" }])).toEqual({ errors: 0, formulas: 3 });
    expect(book[0].cells.slice(2).map(item => item.value)).toEqual(["10", "22", "2.35"]);
    expect(book[0].cells[2].cachedValue).toBe("999");
  });
  test("循环、未知函数和联网函数显示错误，不回用缓存", () => {
    const book = [sheet("Sheet1", [cell("A1", 1, 1, "123", "B1+1"), cell("B1", 1, 2, "456", "A1+1"), cell("C1", 1, 3, "789", "NO_SUCH_FUNCTION(1)"), cell("D1", 1, 4, "999", 'WEBSERVICE("https://example.com")')])];
    expect(recalculateWorkbook(book)).toEqual({ errors: 4, formulas: 4 });
    expect(book[0].cells.every(item => item.formulaError && item.value.startsWith("#"))).toBe(true);
  });
  test("条件和错误处理公式保留布尔、文本和数值类型", () => {
    const book = [sheet("表", [{ ...cell("A1", 1, 1, "TRUE"), valueType: "boolean" }, { ...cell("B1", 1, 2, "名字"), valueType: "text" }, cell("C1", 1, 3, "0", 'IF(A1,B1&"完成",1/0)'), cell("D1", 1, 4, "0", "IFERROR(1/0,42)")])];
    expect(recalculateWorkbook(book)).toEqual({ errors: 0, formulas: 2 });
    expect(book[0].cells.slice(2).map(item => item.value)).toEqual(["名字完成", "42"]);
  });
  test("过大范围和1904日期函数明确失败，普通算术仍可重算", () => {
    const book = [sheet("表", [cell("A1", 1, 1, "0", "SUM(B1:B100001)"), cell("A2", 2, 1, "0", "DATE(2024,1,1)"), cell("A3", 3, 1, "0", "1+2")])];
    expect(recalculateWorkbook(book, [], true)).toEqual({ errors: 2, formulas: 3 });
    expect(book[0].cells[1].formulaError).toContain("1904");
    expect(book[0].cells[2].value).toBe("3");
  });
  test("TEXT按工作簿1904日期系统格式化，不误用1900纪元", () => {
    const book = [sheet("表", [cell("A1", 1, 1, "999", 'TEXT(0,"yyyy-mm-dd")'), cell("A2", 2, 1, "999", 'TEXT(12.5,"0.00")')])];
    expect(recalculateWorkbook(book, [], true)).toEqual({ errors: 0, formulas: 2 });
    expect(book[0].cells.map(item => item.value)).toEqual(["1904-01-01", "12.50"]);
  });
  test("共享整行公式正确平移并从第二行实际重算", () => {
    expect(translateSharedFormula("SUM(1:1,$1:$1,A:A,$B:$B)", 1, 1)).toBe("SUM(2:2,$1:$1,B:B,$B:$B)");
    const book = [sheet("表", [cell("A1", 1, 1, "2"), cell("A2", 2, 1, "5"), cell("B2", 2, 2, "999", "SUM(1:1)"), cell("B3", 3, 2, "999", translateSharedFormula("SUM(1:1)", 1, 0))])];
    expect(recalculateWorkbook(book)).toEqual({ errors: 0, formulas: 2 });
    expect(book[0].cells.slice(2).map(item => item.value)).toEqual(["2", "7"]);
  });
  test("共享公式只平移相对引用，保留字符串、表名和绝对引用", () => {
    expect(translateSharedFormula('SUM(A1,$B1,C$2,$D$3,\'A1\'!E4)&"A1"', 2, 1)).toBe('SUM(B3,$B3,D$2,$D$3,\'A1\'!F6)&"A1"');
    expect(translateSharedFormula("ABC1!A1", 1, 1)).toBe("ABC1!B2");
    expect(translateSharedFormula("A1", -1, 0)).toBe("#REF!");
  });
});
