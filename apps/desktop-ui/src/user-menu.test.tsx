import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { UserMenu, profileInitials } from "./user-menu";
afterEach(cleanup);
it("资料菜单提供外观与设置，不请求企业身份或提供退出账号", () => {
 const open=vi.fn(), theme=vi.fn();
 render(<UserMenu expanded profile={{profileId:"a".repeat(32),displayName:"本地资料"}} theme="dark" onThemeChange={theme} onOpenSettings={open}/>);
 fireEvent.click(screen.getByRole("button",{name:"用户：本地资料"}));
 expect(screen.queryByText("企业额度")).toBeNull(); expect(screen.queryByText("退出")).toBeNull();
 fireEvent.click(screen.getByRole("button",{name:"浅色"})); expect(theme).toHaveBeenCalledWith("light");
 fireEvent.click(screen.getByRole("button",{name:"用户：本地资料"}));
 fireEvent.click(screen.getByRole("menuitem",{name:"设置"})); expect(open).toHaveBeenCalledOnce();
 expect(screen.queryByRole("menu")).toBeNull();
});
it("本地显示名称保持可读缩写",()=>{expect(profileInitials("lance syf")).toBe("LS");expect(profileInitials("本地资料")).toBe("本地");});
