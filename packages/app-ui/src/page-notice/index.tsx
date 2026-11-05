import type { HTMLAttributes, ReactNode } from "react";

export interface PageNoticeProps extends Omit<HTMLAttributes<HTMLDivElement>, "children" | "role"> {
  children: ReactNode;
  icon?: ReactNode;
  actions?: ReactNode;
  dismiss?: ReactNode;
  role?: "status" | "alert";
}

export function PageNotice({ children, icon, actions, dismiss, role = "status", ...element }: PageNoticeProps) {
  return <div {...element} role={role}>{icon}<span>{children}</span>{actions}{dismiss}</div>;
}
