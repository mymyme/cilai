import {cleanup,fireEvent,render,screen,waitFor} from "@testing-library/react";
import {afterEach,beforeEach,expect,test,vi} from "vitest";
import {PersonalCapabilityEditor,personalTemplate} from "./personal-capability-editor";
const host=vi.hoisted(()=>({invoke:vi.fn()}));vi.mock("./host",()=>host);
const initial={project:{creationProjectId:"one",stateRevision:2,targetType:"local_mcp",title:"检查工具",status:"installed"},sourceFiles:[{path:"mcp/server.mjs",content:"first"}],title:"检查工具",businessGoal:"检查文件",requestedPermissions:["personal_file.read"],addedPermissions:[],installedDigest:"sha256:first",candidateDigest:"sha256:first",candidateVersion:"0.1.0",versions:[]};
beforeEach(()=>host.invoke.mockReset());afterEach(cleanup);
test("源码构建不替换安装版本，升级必须核对新权限并显式确认",async()=>{
  const onChanged=vi.fn();host.invoke.mockResolvedValue(initial);
  render(<PersonalCapabilityEditor projectId="one" onClose={()=>{}} onChanged={onChanged}/>);
  fireEvent.change(await screen.findByRole("textbox",{name:"源码 mcp/server.mjs"}),{target:{value:"second"}});
  fireEvent.click(screen.getByRole("checkbox",{name:"确认后写入文件"}));
  const candidate={...initial,project:{...initial.project,stateRevision:3},sourceFiles:[{path:"mcp/server.mjs",content:"second"}],candidateDigest:"sha256:second",candidateVersion:"0.1.1",requestedPermissions:["personal_file.read","personal_file.write_confirmed"],addedPermissions:["personal_file.write_confirmed"]};
  host.invoke.mockResolvedValue(candidate);fireEvent.click(screen.getByRole("button",{name:"保存并构建"}));
  await waitFor(()=>expect(onChanged).toHaveBeenCalledWith(candidate.project,false));
  expect(host.invoke).toHaveBeenLastCalledWith("personal_capability_build",{request:expect.objectContaining({expectedRevision:2,sourceFiles:candidate.sourceFiles,requestedPermissions:candidate.requestedPermissions})});
  fireEvent.click(screen.getByRole("button",{name:"升级"}));expect(screen.getByText(/新增权限：确认后写入文件/)).toBeTruthy();
  expect(host.invoke.mock.calls.some(([name])=>name==="personal_capability_upgrade")).toBe(false);
  host.invoke.mockResolvedValue({...candidate,project:{...candidate.project,stateRevision:4},installedDigest:candidate.candidateDigest});
  fireEvent.click(screen.getByRole("button",{name:"确认"}));
  await waitFor(()=>expect(host.invoke).toHaveBeenLastCalledWith("personal_capability_upgrade",{request:expect.objectContaining({expectedRevision:3,packageDigest:"sha256:second",confirmedPermissions:candidate.requestedPermissions})}));
});
test("源码有未构建修改时关闭需要明确放弃",async()=>{
  host.invoke.mockResolvedValue(initial);const onClose=vi.fn();render(<PersonalCapabilityEditor projectId="one" onClose={onClose} onChanged={()=>{}}/>);
  fireEvent.change(await screen.findByRole("textbox",{name:"源码 mcp/server.mjs"}),{target:{value:"changed"}});fireEvent.click(screen.getByRole("button",{name:"关闭"}));expect(onClose).not.toHaveBeenCalled();fireEvent.click(screen.getByRole("button",{name:"返回编辑"}));expect(onClose).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button",{name:"关闭"}));fireEvent.click(screen.getByRole("button",{name:"放弃修改"}));expect(onClose).toHaveBeenCalledTimes(1);
});
test("模板覆盖静态页面、文件检查和组合页面入口",()=>{
  expect(personalTemplate("web_application").map(file=>file.path)).toEqual(["README.md","web/index.html"]);
  expect(personalTemplate("local_mcp").find(file=>file.path==="mcp/server.mjs")?.content).toContain("selectedFile");
  expect(personalTemplate("web_mcp_application").map(file=>file.path)).toEqual(["README.md","web/index.html","mcp/server.mjs"]);
});

test("纯Web可以显式选择页面权限，本地工具不会出现页面授权",async()=>{
  host.invoke.mockResolvedValue({...initial,project:{...initial.project,targetType:"web_application"},requestedPermissions:[],sourceFiles:[{path:"web/index.html",content:"<h1>笔记</h1>"}]});
  render(<PersonalCapabilityEditor projectId="one" onClose={()=>{}} onChanged={()=>{}}/>);
  const model=await screen.findByRole("checkbox",{name:"页面模型"});
  expect(model).toHaveProperty("checked",false);
  expect(screen.queryByRole("checkbox",{name:"读取选定文件"})).toBeNull();
  expect(screen.queryByRole("checkbox",{name:"页面读取工具"})).toBeNull();
  fireEvent.click(model);fireEvent.click(screen.getByRole("button",{name:"保存并构建"}));
  await waitFor(()=>expect(host.invoke).toHaveBeenCalledWith("personal_capability_build",{request:expect.objectContaining({requestedPermissions:["page_ai.model"]})}));
  cleanup();host.invoke.mockResolvedValue(initial);render(<PersonalCapabilityEditor projectId="one" onClose={()=>{}} onChanged={()=>{}}/>);
  await screen.findByRole("checkbox",{name:"读取选定文件"});
  expect(screen.queryByRole("checkbox",{name:"页面模型"})).toBeNull();
});

test("没有新增权限名的页面声明变化仍需核对，且不会因打开编辑器自动升级",async()=>{
  const page={schemaVersion:1,contextTypes:[{contextType:"note.current",schemaVersion:"1.0",dataSchema:{type:"object",properties:{text:{type:"string",maxLength:200}},additionalProperties:false}}],uiActions:[],model:true,conversationHandoff:false,readTools:[],writeTools:[]};
  const installedPageAi=structuredClone(page);installedPageAi.contextTypes[0].dataSchema.properties.text.maxLength=100;
  const candidate={...initial,project:{...initial.project,targetType:"web_application"},requestedPermissions:["page_ai.context","page_ai.model"],candidateDigest:"sha256:second",candidateVersion:"0.1.1",pageAi:page,installedPageAi};
  host.invoke.mockResolvedValue(candidate);render(<PersonalCapabilityEditor projectId="one" onClose={()=>{}} onChanged={()=>{}}/>);
  await screen.findByText("声明有变化");
  expect(host.invoke.mock.calls.some(([name])=>name==="personal_capability_upgrade")).toBe(false);
  fireEvent.click(screen.getByRole("button",{name:"升级"}));
  expect(screen.getByRole("button",{name:"确认"})).toHaveProperty("disabled",true);
  fireEvent.click(screen.getByRole("checkbox",{name:"我已核对页面 AI 声明变更"}));
  fireEvent.click(screen.getByRole("button",{name:"确认"}));
  await waitFor(()=>expect(host.invoke).toHaveBeenCalledWith("personal_capability_upgrade",{request:expect.objectContaining({expectedRevision:2,packageDigest:"sha256:second",confirmedPermissions:["page_ai.context","page_ai.model"]})}));
});

test("原有包没有页面声明时不自动获得页面授权",async()=>{
  host.invoke.mockResolvedValue({...initial,project:{...initial.project,targetType:"web_application"},requestedPermissions:[]});
  render(<PersonalCapabilityEditor projectId="one" onClose={()=>{}} onChanged={()=>{}}/>);
  await screen.findByRole("checkbox",{name:"页面模型"});
  expect(screen.getAllByRole("checkbox").every(item=>!(item as HTMLInputElement).checked)).toBe(true);
  expect(screen.queryByRole("region",{name:"页面 AI 声明核对"})).toBeNull();
  expect(host.invoke.mock.calls.map(([name])=>name)).toEqual(["personal_capability_editor"]);
});
