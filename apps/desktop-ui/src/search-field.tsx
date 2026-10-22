import type { ReactNode, Ref } from "react";
import { CloseIcon, SearchIcon } from "./icons";
import "./search-field.css";

export function SearchField(props: {
  ariaLabel: string;
  className?: string;
  clearable?: boolean;
  clearLabel?: string;
  disabled?: boolean;
  inputRef?: Ref<HTMLInputElement>;
  placeholder: string;
  trailing?: ReactNode;
  value: string;
  onChange: (value: string) => void;
}) {
  const canClear = props.clearable && props.value.length > 0;
  return <div className={props.className ? `search-field ${props.className}` : "search-field"} data-focus-frame="wrapper">
    <SearchIcon/>
    <input
      aria-label={props.ariaLabel}
      disabled={props.disabled}
      onChange={(event) => props.onChange(event.currentTarget.value)}
      placeholder={props.placeholder}
      ref={props.inputRef}
      type="search"
      value={props.value}/>
    {canClear ? <button aria-label={props.clearLabel ?? `清除${props.ariaLabel}`} className="search-field-clear" onClick={() => props.onChange("")} type="button"><CloseIcon/></button> : props.trailing}
  </div>;
}
