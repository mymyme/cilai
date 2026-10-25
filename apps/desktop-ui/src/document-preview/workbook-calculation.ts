import FormulaParser, { type CellReference, type FormulaValue, type RangeReference } from "fast-formula-parser";
import type { PreviewCell, PreviewSheet } from "./model";

const MAX_REFERENCES = 1_000_000;
const MAX_RANGE_CELLS = 100_000;
const MAX_DEPTH = 128;
export interface DefinedName { name: string; value: string; sheetId?: number; }
const FormulaError = FormulaParser.FormulaError;
const columnIndex = (value: string) => [...value.toUpperCase()].reduce((sum, letter) => sum * 26 + letter.charCodeAt(0) - 64, 0);
function columnName(value: number): string { let result = ""; while (value > 0) { value -= 1; result = String.fromCharCode(65 + value % 26) + result; value = Math.floor(value / 26); } return result; }
export function translateSharedFormula(formula: string, rowOffset: number, columnOffset: number): string {
  return formula.split(/("(?:[^"]|"")*"|'(?:[^']|'')*')/g).map((part, index) => index % 2 ? part : part.replace(/(?<![\w.$])(\$?)([1-9]\d{0,6}):(\$?)([1-9]\d{0,6})(?![\w])/g, (_reference, fixedStart: string, start: string, fixedEnd: string, end: string) => {
    const first = Number(start) + (fixedStart ? 0 : rowOffset); const last = Number(end) + (fixedEnd ? 0 : rowOffset);
    return first < 1 || last < 1 || first > 1048576 || last > 1048576 ? "#REF!" : `${fixedStart}${first}:${fixedEnd}${last}`;
  }).replace(/(?<![\w.])(\$?)([A-Z]{1,3}):(\$?)([A-Z]{1,3})(?![\w!])/gi, (_reference, fixedStart: string, start: string, fixedEnd: string, end: string) => {
    const first = columnIndex(start) + (fixedStart ? 0 : columnOffset); const last = columnIndex(end) + (fixedEnd ? 0 : columnOffset);
    return first < 1 || last < 1 || first > 16384 || last > 16384 ? "#REF!" : `${fixedStart}${columnName(first)}:${fixedEnd}${columnName(last)}`;
  }).replace(/(?<![\w.])([\$]?)([A-Z]{1,3})([\$]?)([1-9]\d{0,6})(?![\w!]|\s*\()/gi, (reference, fixedColumn: string, column: string, fixedRow: string, row: string) => {
    if (columnIndex(column) > 16384) return reference;
    const nextColumn = columnIndex(column) + (fixedColumn ? 0 : columnOffset); const nextRow = Number(row) + (fixedRow ? 0 : rowOffset);
    return nextColumn < 1 || nextColumn > 16384 || nextRow < 1 || nextRow > 1048576 ? "#REF!" : `${fixedColumn}${columnName(nextColumn)}${fixedRow}${nextRow}`;
  })).join("");
}
function nameReference(value: string, sheet: string): CellReference | RangeReference | null {
  const match = /^(?:(?:'((?:[^']|'')+)'|([^!]+))!)?\$?([A-Z]{1,3})\$?(\d+)(?::\$?([A-Z]{1,3})\$?(\d+))?$/i.exec(value);
  if (!match) return null;
  const target = match[1]?.replaceAll("''", "'") ?? match[2] ?? sheet;
  return match[5] ? { sheet: target, from: { row: Number(match[4]), col: columnIndex(match[3]) }, to: { row: Number(match[6]), col: columnIndex(match[5]) } } : { sheet: target, row: Number(match[4]), col: columnIndex(match[3]) };
}
function rawValue(cell: PreviewCell | undefined): FormulaValue {
  if (!cell || cell.value === "") return null;
  if (cell.valueType === "boolean") return cell.value === "TRUE";
  if (cell.valueType === "error") return new FormulaError(cell.value);
  if (cell.valueType === "number") { const value = Number(cell.value); return Number.isFinite(value) ? value : new FormulaError("#VALUE!"); }
  return cell.value;
}
export function recalculateWorkbook(sheets: PreviewSheet[], names: DefinedName[] = [], date1904 = false): { errors: number; formulas: number } {
  const bySheet = new Map(sheets.map(sheet => [sheet.name.toLocaleLowerCase(), sheet]));
  const cells = new Map(sheets.map(sheet => [sheet.name, new Map(sheet.cells.map(cell => [`${cell.row}:${cell.column}`, cell]))]));
  const results = new Map<PreviewCell, FormulaValue>(); const visiting = new Set<PreviewCell>();
  const parsers: FormulaParser[] = []; let references = 0; let formulas = 0; let errors = 0;
  const evaluate = (reference: CellReference, depth: number): FormulaValue => {
    if (++references > MAX_REFERENCES || depth > MAX_DEPTH) return new FormulaError("#LIMIT!", "公式依赖超过预览计算上限");
    const sheet = bySheet.get((reference.sheet ?? "").toLocaleLowerCase());
    if (!sheet || !Number.isInteger(reference.row) || !Number.isInteger(reference.col) || reference.row < 1 || reference.col < 1 || reference.row > 1048576 || reference.col > 16384) return FormulaError.REF;
    const cell = cells.get(sheet.name)?.get(`${reference.row}:${reference.col}`);
    if (!cell?.formula) return rawValue(cell);
    if (results.has(cell)) return results.get(cell)!;
    if (visiting.has(cell)) return new FormulaError("#CYCLE!", "公式存在循环引用");
    visiting.add(cell);
    let result: FormulaValue;
    try {
      if (cell.formula.length > 32768) throw new Error("公式超过长度上限");
      if (cell.formulaError) throw new Error(cell.formulaError);
      if (date1904 && /\b(?:DATE|DATEVALUE|DAY|DAYS|DAYS360|EDATE|EOMONTH|HOUR|ISOWEEKNUM|MINUTE|MONTH|NETWORKDAYS(?:\.INTL)?|NOW|SECOND|TIMEVALUE|TODAY|WEEKDAY|WEEKNUM|WORKDAY(?:\.INTL)?|YEAR|YEARFRAC)\s*\(/i.test(cell.formula.replace(/"(?:[^"]|"")*"/g, '""'))) throw new FormulaError("#UNSUPPORTED!", "1904 日期系统的日期函数尚不支持");
      const parser = parsers[depth] ??= new FormulaParser({
        onCell: ref => evaluate(ref, depth + 1),
        onRange: ref => {
          const count = (ref.to.row - ref.from.row + 1) * (ref.to.col - ref.from.col + 1);
          if (!Number.isInteger(count) || count < 0 || count > MAX_RANGE_CELLS) throw new Error("公式范围超过预览计算上限");
          const values: FormulaValue[][] = [];
          for (let row = ref.from.row; row <= ref.to.row; row += 1) {
            const line: FormulaValue[] = [];
            for (let col = ref.from.col; col <= ref.to.col; col += 1) line.push(evaluate({ sheet: ref.sheet, row, col }, depth + 1));
            values.push(line);
          }
          return values;
        },
        onVariable: (name, scope) => {
          const sheetIndex = sheets.findIndex(sheet => sheet.name === scope);
          const defined = names.find(item => item.name.toUpperCase() === name.toUpperCase() && item.sheetId === sheetIndex) ?? names.find(item => item.name.toUpperCase() === name.toUpperCase() && item.sheetId === undefined);
          return defined ? nameReference(defined.value, scope) : null;
        },
        functions: { TEXT: (...args) => {
          if (args.length !== 2) return new FormulaError("#VALUE!");
          const scalar = (argument: unknown) => {
            if (!argument || typeof argument !== "object" || !("value" in argument) || "isArray" in argument && argument.isArray) throw new FormulaError("#VALUE!");
            if (argument.value instanceof FormulaError) throw argument.value;
            return argument.value;
          };
          const value = Number(scalar(args[0])); const format = scalar(args[1]);
          if (!Number.isFinite(value) || typeof format !== "string") return new FormulaError("#VALUE!");
          return FormulaParser.SSF.format(format, value, { date1904 });
        }, WEBSERVICE: () => new FormulaError("#UNSUPPORTED!", "预览不执行联网公式"), FILTERXML: () => new FormulaError("#UNSUPPORTED!", "此公式尚不支持") },
      });
      result = parser.parse(cell.formula.replace(/^=/, ""), { sheet: sheet.name, row: cell.row, col: cell.column });
      if (!(result instanceof FormulaError) && !(typeof result === "string" || typeof result === "boolean" || typeof result === "number" && Number.isFinite(result) || result === null)) result = new FormulaError("#VALUE!", "公式未产生有效单元格值");
    } catch (error) {
      result = error instanceof FormulaError ? error : new FormulaError("#ERROR!", error instanceof Error ? error.message : "公式计算失败");
    }
    visiting.delete(cell); results.set(cell, result); return result;
  };
  for (const sheet of sheets) for (const cell of sheet.cells) if (cell.formula) {
    formulas += 1; const result = evaluate({ sheet: sheet.name, row: cell.row, col: cell.column }, 0);
    if (result instanceof FormulaError) { cell.value = result.error; cell.formulaError = result.message || "公式计算失败"; errors += 1; }
    else {
      cell.value = result === null ? "" : typeof result === "boolean" ? result ? "TRUE" : "FALSE" : cell.numberFormat ? FormulaParser.SSF.format(cell.numberFormat, result, { date1904 }) : String(result);
      cell.calculatedValue = result; cell.formulaError = undefined;
    }
  }
  return { errors, formulas };
}
