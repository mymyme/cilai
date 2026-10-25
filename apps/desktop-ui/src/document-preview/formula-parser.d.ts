declare module "fast-formula-parser" {
  export type FormulaValue = string | number | boolean | null | FormulaError;
  export interface CellReference { sheet?: string; row: number; col: number; }
  export interface RangeReference { sheet?: string; from: { row: number; col: number }; to: { row: number; col: number }; }
  export class FormulaError extends Error { constructor(code: string, message?: string); error: string; static REF: FormulaError; static NAME: FormulaError; }
  export default class FormulaParser {
    static FormulaError: typeof FormulaError;
    static SSF: { format(format: string | number, value: string | number | boolean, options?: { date1904?: boolean }): string };
    constructor(options: { onCell: (reference: CellReference) => FormulaValue; onRange: (reference: RangeReference) => FormulaValue[][]; onVariable: (name: string, sheet: string) => CellReference | RangeReference | null; functions?: Record<string, (...args: unknown[]) => FormulaValue> });
    parse(formula: string, position: CellReference): FormulaValue;
  }
}
