import type { ReactNode } from "react";

export function WorkbenchFilterGroup<Value extends string>({ label, options, value, onChange, className, compact }: {
  label: string;
  options: ReadonlyArray<readonly [Value, ReactNode]>;
  value: Value;
  onChange: (value: Value) => void;
  className?: string;
  compact?: boolean;
}) {
  return <div aria-label={label} className={className ? `workbench-filter-group ${className}` : "workbench-filter-group"} data-compact={compact || undefined} role="group">
    {options.map(([key, text]) => <button aria-pressed={value === key} key={key} onClick={() => onChange(key)} type="button">{text}</button>)}
  </div>;
}
