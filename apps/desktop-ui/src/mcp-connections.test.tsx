import {cleanup,fireEvent,render,screen,waitFor,within} from "@testing-library/react";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import type {McpConnection,McpConnectionChange,McpValidationRecord} from "./generated/protocol";
import {McpConnectionSettings,mcpError} from "./mcp-connections";
const invoke=vi.hoisted(()=>vi.fn());
vi.mock("./host",()=>({invoke}));
afterEach(cleanup);
const connection=():McpConnection=>({profileId:"1".repeat(32),connectionId:"2".repeat(32),revision:1,authGeneration:0,
  config:{name:"个人工具",source:{kind:"remote_http",url:"https://tools.example.invalid/mcp"},authMode:"none",environmentVariables:[],network:{mode:"direct",proxyUrl:null}},
  credentialRef:null,enabled:false,deleted:false,approvedToolNames:[],purposes:[],approvedCatalogDigest:null,validation:null,updatedAt:1});
const validation=():McpValidationRecord=>({validationId:"validation-one",connectionId:"2".repeat(32),revision:1,state:"passed",reasonCode:null,
  protocolVersion:"2025-03-26",serverName:"本机测试服务",serverVersion:"1",sourceFingerprint:"digest",tools:[{name:"echo",description:"<img src=x onerror=private>仅为文本",inputSchema:{type:"object"},annotations:null}],catalogDigest:"catalog-one",validatedAt:1});
let items:McpConnection[],operations:McpConnectionChange[];
beforeEach(()=>{items=[connection()];operations=[];invoke.mockReset();invoke.mockImplementation(async(command:string)=>{
  if(command==="mcp_connection_list")return structuredClone(items);
  if(command==="mcp_connection_changes")return structuredClone(operations);
  throw new Error("unexpected operation");
});});
it("mount only reads owned metadata without validating or enabling tools",async()=>{
  render(<McpConnectionSettings/>);await screen.findByText("个人工具");
  expect(invoke.mock.calls.map(call=>call[0])).toEqual(["mcp_connection_list","mcp_connection_changes"]);
  expect(screen.getByText("已停用")).toBeTruthy();
});
it("source validation requires confirmation and cancel never contacts the service",async()=>{
  render(<McpConnectionSettings/>);fireEvent.click(await screen.findByRole("button",{name:"验证"}));
  expect(screen.getByRole("dialog").textContent).toContain("https://tools.example.invalid/mcp");
  fireEvent.click(screen.getByRole("button",{name:"取消"}));
  expect(invoke.mock.calls.some(call=>call[0]==="mcp_connection_validate")).toBe(false);
});
it("actual validation is followed by exact tool and purpose approval",async()=>{
  const original=invoke.getMockImplementation()!;
  invoke.mockImplementation(async(command:string,args:unknown)=>{
    if(command==="mcp_connection_validate"){items[0].validation=validation();return validation();}
    if(command==="mcp_connection_grant"){items[0].enabled=true;return structuredClone(items[0]);}
    return original(command,args);
  });
  render(<McpConnectionSettings/>);fireEvent.click(await screen.findByRole("button",{name:"验证"}));fireEvent.click(screen.getByRole("button",{name:"开始验证"}));
  await screen.findByRole("checkbox",{name:"echo"});
  expect(document.querySelector("img")).toBeNull();
  expect((screen.getByRole("button",{name:"确认并启用"}) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole("checkbox",{name:"echo"}));fireEvent.click(screen.getByRole("checkbox",{name:"前台对话"}));
  fireEvent.click(screen.getByRole("button",{name:"确认并启用"}));
  await screen.findByText("已启用。");
  expect(invoke).toHaveBeenCalledWith("mcp_connection_validate",{request:{connectionId:"2".repeat(32),expectedRevision:1,sourceConsent:true}});
  expect(invoke).toHaveBeenCalledWith("mcp_connection_grant",{request:{connectionId:"2".repeat(32),expectedRevision:1,validationId:"validation-one",approvedToolNames:["echo"],purposes:["foreground"],sourceConsent:true}});
});
it("local command confirmation displays exact arguments and permissions before launch",async()=>{
  items[0].config.source={kind:"local_command",program:"/approved/tool",args:["--stdio"],cwd:"/approved/work",permissions:{readRoots:["/approved/read"],writeRoots:[],network:false,childProcesses:false}};
  render(<McpConnectionSettings/>);fireEvent.click(await screen.findByRole("button",{name:"验证"}));
  const dialog=screen.getByRole("dialog");
  for(const value of ["--stdio","/approved/work","/approved/read","未授予","禁止"])expect(dialog.textContent).toContain(value);
  expect(invoke.mock.calls.some(call=>call[0]==="mcp_connection_validate")).toBe(false);
});
it("password is cleared immediately and only sent in the dedicated one-time secret map",async()=>{
  items=[];
  render(<McpConnectionSettings/>);await waitFor(()=>expect((screen.getByRole("button",{name:"添加"}) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(screen.getByRole("button",{name:"添加"}));
  fireEvent.change(screen.getByLabelText("名称"),{target:{value:"自己的服务"}});
  fireEvent.change(screen.getByLabelText("服务地址"),{target:{value:"https://tools.example.invalid/mcp"}});
  fireEvent.click(screen.getByRole("combobox",{name:"认证"}));fireEvent.click(screen.getByRole("option",{name:"Bearer"}));
  const secret=screen.getByLabelText("token") as HTMLInputElement;
  fireEvent.change(secret,{target:{value:"synthetic-only-key"}});
  fireEvent.click(screen.getByRole("button",{name:"保存"}));
  expect(secret.value).toBe("");
  await screen.findByRole("alert");
  const request=invoke.mock.calls.find(call=>call[0]==="mcp_connection_save")?.[1].request;
  expect(request.secrets).toEqual({token:"synthetic-only-key"});
  expect(JSON.stringify(request.prepare)).not.toContain("synthetic-only-key");
  expect(invoke.mock.calls.filter(call=>call[0]==="mcp_connection_save")).toHaveLength(1);
  expect((screen.getByRole("button",{name:"保存"}) as HTMLButtonElement).disabled).toBe(true);
});
it("failed disable preserves state and requires readonly refresh before another mutation",async()=>{
  items[0].enabled=true;
  render(<McpConnectionSettings/>);fireEvent.click(await screen.findByRole("button",{name:"停用"}));
  await screen.findByRole("alert");
  expect(screen.getByText("已启用")).toBeTruthy();
  expect((screen.getByRole("button",{name:"停用"}) as HTMLButtonElement).disabled).toBe(true);
  expect(invoke.mock.calls.filter(call=>call[0]==="mcp_connection_disable")).toHaveLength(1);
  fireEvent.click(screen.getByRole("button",{name:"刷新"}));
  await waitFor(()=>expect((screen.getByRole("button",{name:"停用"}) as HTMLButtonElement).disabled).toBe(false));
});
it("credential cleanup requires a separate confirmation and does not recreate a connection",async()=>{
  const ref={service:"cn.cilai.personal.workbench.vault.mcp",account:`${"1".repeat(32)}:${"2".repeat(32)}:1`};
  operations=[{operationId:"operation-one",state:"aborted",expectedRevision:null,connection:connection(),credentialAction:"replace",cleanupReferences:[ref],cleanedReferences:[]}];
  render(<McpConnectionSettings/>);fireEvent.click(await screen.findByRole("button",{name:"清理历史凭据"}));
  const dialog=screen.getByRole("dialog",{name:"清理历史凭据？"});expect(dialog.textContent).toContain("个人工具");expect(dialog.textContent).toContain("工作台本地加密凭据");expect(dialog.textContent).not.toContain(ref.account);
  fireEvent.click(within(dialog).getByRole("button",{name:"取消"}));
  expect(invoke.mock.calls.some(call=>call[0]==="mcp_connection_cleanup")).toBe(false);
  fireEvent.click(screen.getByRole("button",{name:"清理历史凭据"}));fireEvent.click(within(screen.getByRole("dialog")).getByRole("button",{name:"清理"}));
  await screen.findByRole("alert");
  expect(invoke).toHaveBeenCalledWith("mcp_connection_cleanup",{request:{operationId:"operation-one",credentialRef:ref}});
  expect(invoke.mock.calls.some(call=>call[0]==="mcp_connection_save")).toBe(false);
});
it("metadata read failure does not become an empty successful configuration",async()=>{
  invoke.mockRejectedValue(new Error("private-token-raw-error"));
  render(<McpConnectionSettings/>);await screen.findByRole("alert");
  expect(screen.queryByText("尚无工具连接。")).toBeNull();
  expect((screen.getByRole("button",{name:"添加"}) as HTMLButtonElement).disabled).toBe(true);
  expect(document.body.textContent).not.toContain("private-token-raw-error");
});
it("unknown service errors never echo arbitrary payload or secret-like error tokens",()=>{
  expect(mcpError(new Error("mcp_unknown_private_secret_abc token=another-private-value"))).toBe("操作未确认，请刷新状态后核对。");
});


it.each(["cn.cilai.personal.workbench.mcp", "cn.cilai.personal.workbench.vault.mcp.extra"])("旧MCP引用%s要求重新输入，空输入不能继续保留旧系统凭据", async (service) => {
  items[0].config.authMode="bearer";
  items[0].credentialRef={service,account:"private-mcp-reference"};
  render(<McpConnectionSettings/>);fireEvent.click(await screen.findByRole("button",{name:"编辑"}));
  const editor=within(screen.getByRole("region",{name:"编辑工具连接"}));
  expect(editor.getByText("需重新保存到工作台")).toBeTruthy();
  const input=editor.getByLabelText("token");
  expect(input).toHaveProperty("value","");expect(input).toHaveProperty("required",true);
  expect(editor.queryByText(/留空保留/)).toBeNull();
  expect(document.body.textContent).not.toContain("private-mcp-reference");
  fireEvent.click(editor.getByRole("button",{name:"保存"}));
  expect(invoke.mock.calls.some(call=>call[0]==="mcp_connection_save")).toBe(false);
  fireEvent.change(input,{target:{value:"synthetic-mcp-reentry"}});fireEvent.click(editor.getByRole("button",{name:"保存"}));
  await screen.findByRole("alert");
  const calls=invoke.mock.calls.filter(call=>call[0]==="mcp_connection_save");
  expect(calls).toHaveLength(1);
  expect(calls[0][1].request.prepare).toMatchObject({connectionId:items[0].connectionId,expectedRevision:items[0].revision,credentialAction:"replace"});
  expect(calls[0][1].request.secrets).toEqual({token:"synthetic-mcp-reentry"});
  expect(JSON.stringify(calls[0][1].request.prepare)).not.toContain("synthetic-mcp-reentry");
  expect(input).toHaveProperty("value","");
});

it("本地MCP凭据只显示登记位置，不读取旧值，留空保存仍用同一修订", async () => {
  items[0].config.authMode="bearer";
  items[0].credentialRef={service:"cn.cilai.personal.workbench.vault.mcp",account:"private-mcp-vault-account"};
  render(<McpConnectionSettings/>);fireEvent.click(await screen.findByRole("button",{name:"编辑"}));
  expect(screen.getByText("本地凭据已登记。留空保留原凭据。")).toBeTruthy();
  expect(screen.getByLabelText("token")).toHaveProperty("value","");
  expect(screen.getByLabelText("token")).toHaveProperty("required",false);
  expect(document.body.textContent).not.toContain("private-mcp-vault-account");
  fireEvent.click(screen.getByRole("button",{name:"保存"}));await screen.findByRole("alert");
  const request=invoke.mock.calls.find(call=>call[0]==="mcp_connection_save")?.[1].request;
  expect(request.prepare).toMatchObject({connectionId:items[0].connectionId,expectedRevision:items[0].revision,credentialAction:"keep"});
  expect(request.secrets).toBeNull();
  expect(invoke.mock.calls.some(call=>call[0]==="mcp_credential_read")).toBe(false);
});

it("旧系统MCP清理仅显示手工未完成，不泄露引用或发出清理请求", async () => {
  const ref={service:"cn.cilai.personal.workbench.mcp",account:"private-system-account"};
  operations=[{operationId:"legacy-operation",state:"aborted",expectedRevision:null,connection:connection(),credentialAction:"replace",cleanupReferences:[ref],cleanedReferences:[]}];
  render(<McpConnectionSettings/>);
  await screen.findByText("旧系统凭据需手工清理，工作台未确认删除。");
  expect(screen.queryByRole("button",{name:"清理历史凭据"})).toBeNull();
  expect(document.body.textContent).not.toContain(ref.account);
  expect(invoke.mock.calls.every(call=>["mcp_connection_list","mcp_connection_changes"].includes(call[0]))).toBe(true);
});

it.each(["missing-cleaned-reference","other-profile","other-operation"])("MCP清理返回%s时不冒充已确认，不自动重做", async (reason) => {
  const ref={service:"cn.cilai.personal.workbench.vault.mcp",account:"private-cleanup-reference"};
  const operation:McpConnectionChange={operationId:"cleanup-operation",state:"aborted",expectedRevision:null,connection:connection(),credentialAction:"replace",cleanupReferences:[ref],cleanedReferences:[]};
  operations=[operation];
  const original=invoke.getMockImplementation()!;
  invoke.mockImplementation(async(command:string,args:unknown)=>{
    if(command==="mcp_connection_cleanup")return {...operation,operationId:reason==="other-operation"?"another-operation":operation.operationId,
      connection:{...operation.connection,profileId:reason==="other-profile"?"another-profile":operation.connection.profileId},cleanedReferences:reason==="missing-cleaned-reference"?[]:[ref]};
    return original(command,args);
  });
  render(<McpConnectionSettings/>);fireEvent.click(await screen.findByRole("button",{name:"清理历史凭据"}));
  fireEvent.click(within(screen.getByRole("dialog")).getByRole("button",{name:"清理"}));
  await screen.findByText("本地凭据删除尚未确认。");
  expect(screen.queryByText("凭据清理已确认。")).toBeNull();
  expect(invoke.mock.calls.filter(call=>call[0]==="mcp_connection_cleanup")).toHaveLength(1);
  expect(document.body.textContent).not.toContain(ref.account);
});

it("MCP清理仅在同操作同资料的exact引用有确认回执后显示成功", async () => {
  const ref={service:"cn.cilai.personal.workbench.vault.mcp",account:"private-cleanup-reference"};
  const operation:McpConnectionChange={operationId:"cleanup-operation",state:"aborted",expectedRevision:null,connection:connection(),credentialAction:"replace",cleanupReferences:[ref],cleanedReferences:[]};
  operations=[operation];
  const original=invoke.getMockImplementation()!;
  invoke.mockImplementation(async(command:string,args:unknown)=>{
    if(command==="mcp_connection_cleanup"){operations=[{...operation,cleanedReferences:[ref]}];return operations[0];}
    return original(command,args);
  });
  render(<McpConnectionSettings/>);fireEvent.click(await screen.findByRole("button",{name:"清理历史凭据"}));
  fireEvent.click(within(screen.getByRole("dialog")).getByRole("button",{name:"清理"}));
  await screen.findByText("凭据清理已确认。");
  expect(screen.queryByRole("button",{name:"清理历史凭据"})).toBeNull();
  expect(invoke.mock.calls.filter(call=>call[0]==="mcp_connection_cleanup")).toHaveLength(1);
  expect(invoke.mock.calls.some(call=>call[0]==="mcp_connection_save")).toBe(false);
});

it("本地凭据重录和旧系统手工清理错误提供明确中文，不回显秘密", () => {
  expect(mcpError(new Error("mcp_credential_reentry_required"))).toBe("请重新保存到工作台本地凭据库。");
  expect(mcpError(new Error("mcp_credential_legacy_cleanup_required"))).toBe("旧系统凭据需手工清理，工作台未确认删除。");
  expect(mcpError(new Error("unknown_service_error token=private-value"))).toBe("操作未确认，请刷新状态后核对。");
});


it("旧系统MCP凭据的keep操作必须取消后重存，不能直接继续", async () => {
  const pending=connection();pending.config.authMode="bearer";
  pending.credentialRef={service:"cn.cilai.personal.workbench.mcp",account:"private-legacy-keep"};
  operations=[{operationId:"keep-operation",state:"prepared",expectedRevision:1,connection:pending,credentialAction:"keep",cleanupReferences:[],cleanedReferences:[]}];
  render(<McpConnectionSettings/>);fireEvent.click(await screen.findByRole("button",{name:"继续保存"}));
  const editor=within(screen.getByRole("region",{name:"继续保存工具连接"}));
  expect(editor.getByText("请取消此操作后重新输入凭据。")).toBeTruthy();
  expect(editor.getByRole("button",{name:"继续保存"})).toHaveProperty("disabled",true);
  expect(editor.getByRole("button",{name:"关闭"})).toHaveProperty("disabled",false);
  fireEvent.click(editor.getByRole("button",{name:"继续保存"}));
  expect(invoke.mock.calls.some(call=>call[0]==="mcp_connection_continue_save")).toBe(false);
  expect(document.body.textContent).not.toContain("private-legacy-keep");
});


it("切换MCP编辑连接清空尚未提交的输入，不把密钥附到另一连接", async () => {
  const first=connection();first.config.authMode="bearer";
  first.credentialRef={service:"cn.cilai.personal.workbench.mcp",account:"private-first-reference"};
  const second={...first,connectionId:"4".repeat(32),config:{...first.config,name:"另一工具"}};
  items=[first,second];
  render(<McpConnectionSettings/>);await screen.findByText("另一工具");
  fireEvent.click(screen.getAllByRole("button",{name:"编辑"})[0]);
  fireEvent.change(screen.getByLabelText("token"),{target:{value:"synthetic-first-unsaved-key"}});
  fireEvent.click(screen.getAllByRole("button",{name:"编辑"})[1]);
  expect(screen.getByLabelText("token")).toHaveProperty("value","");
  expect(screen.getByLabelText("名称")).toHaveProperty("value","另一工具");
  fireEvent.click(screen.getAllByRole("button",{name:"编辑"})[0]);
  expect(screen.getByLabelText("token")).toHaveProperty("value","");
  expect(document.body.innerHTML).not.toContain("synthetic-first-unsaved-key");
  expect(invoke.mock.calls.some(call=>call[0]==="mcp_connection_save")).toBe(false);
});
