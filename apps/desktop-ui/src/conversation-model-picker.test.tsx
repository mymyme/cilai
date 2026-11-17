import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { ConversationModelPicker } from "./conversation-model-picker";
import type { AiModelInfo } from "./ai-runtime";
afterEach(cleanup);
const model=(connectionId:string,effort:"low"|"high"="low"):AiModelInfo=>({connectionId,connectionName:connectionId,modelId:"same",label:"同名模型",provider:"personal",description:"",source:"personal",contextWindowTokens:32768,inputModalities:["text"],supportedReasoningEfforts:["low","high"],defaultReasoningEffort:effort});
const common={onOpen:()=>{},historical:false,loading:false,disabled:false};
test("同名模型按个人连接分别选择，新选择保持跟随默认",()=>{
  const select=vi.fn();render(<ConversationModelPicker {...common} models={[model("A"),model("B")]} selectedModelId={null} selectedConnectionId={null} selectedReasoningEffort={null} onSelectModel={select}/>);
  fireEvent.click(screen.getByRole("button",{name:/选择模型：/}));
  fireEvent.click(screen.getByRole("menuitemradio",{name:/B · 同名模型/}));
  expect(select).toHaveBeenLastCalledWith("same",null,"B");
});
test("显式档位随目录刷新保持，恢复默认提交空选择",()=>{
  const select=vi.fn();const props={...common,selectedModelId:"same",selectedConnectionId:"A",selectedReasoningEffort:"high" as const,onSelectModel:select};
  const view=render(<ConversationModelPicker {...props} models={[model("A")]}/>);
  view.rerender(<ConversationModelPicker {...props} models={[model("A","high")]}/>);
  expect(screen.getByRole("button",{name:/同名模型 较高/})).toBeTruthy();expect(select).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button",{name:/选择模型：/}));
  fireEvent.click(screen.getByRole("button",{name:"恢复模型默认深度"}));
  expect(select).toHaveBeenLastCalledWith("same",null,"A");
});
