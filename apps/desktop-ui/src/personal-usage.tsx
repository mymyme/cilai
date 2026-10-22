import { useEffect, useMemo, useState } from "react";
import { invoke } from "./host";
import { aiErrorMessage } from "./ai-runtime";
import "./personal-usage.css";
import {Select} from "./select";

const purposes = {foreground:"前台对话",automation:"自动化",review:"自动审核",memory:"后台记忆",page_ai:"页面 AI",classification:"会话整理",unknown:"用途未提供"};
type Purpose = keyof typeof purposes;
const counters = [
  ["inputTokens","输入"], ["cachedInputTokens","缓存输入"], ["cacheWriteInputTokens","缓存写入"],
  ["outputTokens","输出"], ["reasoningOutputTokens","推理输出"], ["totalTokens","总计"],
] as const;
type Counter = typeof counters[number][0];
type Tokens = Partial<Record<Counter,number|null>> & {source:"official_runtime_usage"|"upstream_responses_usage"};
type Cost = {amount:string;currency:string;source:"upstream.responses.usage.cost"};
type UsageRow = {runId:string;purpose:Purpose;phase:string;connectionId:string|null;modelId:string|null;status:string|null;
  startedAt:number;completedAt:number|null;tokens:Tokens|null;cost:Cost|null;recordSource?:string;
  sourceProfileId?:string|null;configRevision?:number|null;authGeneration?:number|null;readOnlyAfterRestore?:boolean;
  contributesToTotals?:boolean;duplicateOfRequestId?:string};
type UsageView = {stateRevision:number;rows:UsageRow[];coverage:Array<{purpose:Purpose;source:string}>;costSource:null;priceCheckedAt:null};
type Metric = {value:number;observed:number;overflow:boolean};
type CostTotal = {units:bigint;scale:number;observed:number;latestAt:number};
type Group = {key:string;purpose:Purpose;connectionId:string|null;modelId:string|null;records:number;observed:number;
  metrics:Record<Counter,Metric>;costs:Map<string,CostTotal>;costObserved:number;restored:number;unconfirmed:number;duplicates:number;latestAt:number};
const count = (value:unknown):value is number => typeof value === "number" && Number.isSafeInteger(value) && value>=0;
const decimal = (value:unknown):value is string => typeof value==="string" && value.length<=80 && /^(?:0|[1-9][0-9]*)(?:\.[0-9]+)?$/.test(value);
const validCost = (value:Cost|null) => value===null || !!value && decimal(value.amount) && /^[A-Z]{3}$/.test(value.currency)
  && value.source==="upstream.responses.usage.cost";
function validTokens(value:Tokens|null):boolean {
  if(value===null)return true;
  if(!value)return false;
  if(value.source==="official_runtime_usage")return [value.inputTokens,value.cachedInputTokens,value.outputTokens].every(count)
    && [value.cacheWriteInputTokens,value.reasoningOutputTokens,value.totalTokens].every(item=>item==null);
  return value.source==="upstream_responses_usage" && counters.every(([key])=>value[key]===null || count(value[key]));
}
function checkedView(value:UsageView):UsageView {
  if (!value || !count(value.stateRevision) || !Array.isArray(value.rows) || !Array.isArray(value.coverage)
    || value.costSource!==null || value.priceCheckedAt!==null
    || value.coverage.some(item=>!item || !Object.hasOwn(purposes,item.purpose) || typeof item.source!=="string")
    || value.rows.some(row=>!row || !Object.hasOwn(purposes,row.purpose) || typeof row.runId!=="string" || typeof row.phase!=="string"
      || (row.status!==null && typeof row.status!=="string")
      || (row.connectionId!==null && typeof row.connectionId!=="string") || (row.modelId!==null && typeof row.modelId!=="string")
      || !count(row.startedAt) || (row.completedAt!==null && !count(row.completedAt)) || !validCost(row.cost) || !validTokens(row.tokens)
      || (row.recordSource!==undefined && typeof row.recordSource!=="string")
      || (row.sourceProfileId!=null && typeof row.sourceProfileId!=="string")
      || (row.configRevision!=null && !count(row.configRevision)) || (row.authGeneration!=null && !count(row.authGeneration))
      || (row.readOnlyAfterRestore!==undefined && typeof row.readOnlyAfterRestore!=="boolean")
      || (row.contributesToTotals!==undefined && typeof row.contributesToTotals!=="boolean")
      || (row.duplicateOfRequestId!==undefined && (typeof row.duplicateOfRequestId!=="string" || !row.duplicateOfRequestId || row.duplicateOfRequestId===row.runId))
      || (row.contributesToTotals===false && (row.recordSource!=="persisted_request_metadata" || !row.duplicateOfRequestId))
      || (row.duplicateOfRequestId!==undefined && row.contributesToTotals!==false)
      || (row.recordSource==="persisted_request_metadata" && (row.phase!=="request" || !["review","memory"].includes(row.purpose)
        || !["completed","failed","unconfirmed"].includes(row.status??"") || typeof row.sourceProfileId!=="string"
        || !count(row.configRevision) || !count(row.authGeneration)))
      || (row.tokens?.source==="upstream_responses_usage" && row.recordSource!=="persisted_request_metadata")
      || (row.cost!==null && row.recordSource!=="persisted_request_metadata"))) throw new Error("用量记录无效，请重新读取。");
  const requests=new Map<string,UsageRow>();
  for(const row of value.rows.filter(item=>item.recordSource==="persisted_request_metadata")){
    if(requests.has(row.runId))throw new Error("用量记录无效，请重新读取。");
    requests.set(row.runId,row);
  }
  for(const row of requests.values()){
    if(row.contributesToTotals!==false)continue;
    const original=requests.get(row.duplicateOfRequestId??"");
    const identity=["purpose","connectionId","modelId","sourceProfileId","configRevision","authGeneration"] as const;
    if(!original || original.contributesToTotals===false || identity.some(key=>row[key]!==original[key])
      || (row.tokens===null)!==(original.tokens===null)
      || row.tokens?.source!==original.tokens?.source || counters.some(([key])=>row.tokens?.[key]!==original.tokens?.[key])
      || row.cost?.amount!==original.cost?.amount || row.cost?.currency!==original.cost?.currency || row.cost?.source!==original.cost?.source)
      throw new Error("用量记录无效，请重新读取。");
  }
  return value;
}
function addCost(total:CostTotal|undefined,amount:string,observedAt:number):CostTotal {
  const [whole,fraction=""]=amount.split(".");
  const scale=Math.max(total?.scale??0,fraction.length);
  const units=BigInt(whole+fraction)*10n**BigInt(scale-fraction.length)
    +(total?.units??0n)*10n**BigInt(scale-(total?.scale??0));
  return {units,scale,observed:(total?.observed??0)+1,latestAt:Math.max(total?.latestAt??0,observedAt)};
}
const timeText=(timestamp:number)=>Number.isNaN(new Date(timestamp).valueOf())?"未提供":new Date(timestamp).toLocaleString();
function costText(total:CostTotal):string {
  const digits=total.units.toString().padStart(total.scale+1,"0");
  if(total.scale===0)return digits;
  const fraction=digits.slice(-total.scale).replace(/0+$/,"");
  return digits.slice(0,-total.scale)+(fraction?"."+fraction:"");
}
export function groupUsage(rows:UsageRow[]):Group[] {
  const groups=new Map<string,Group>();
  for (const row of rows) {
    const key=JSON.stringify([row.purpose,row.connectionId,row.modelId,row.sourceProfileId??null,
      row.recordSource==="persisted_request_metadata"?"request":"run"]);
    let group=groups.get(key);
    if (!group) {
      group={key,purpose:row.purpose,connectionId:row.connectionId,modelId:row.modelId,records:0,observed:0,
        metrics:Object.fromEntries(counters.map(([name])=>[name,{value:0,observed:0,overflow:false}])) as Record<Counter,Metric>,
        costs:new Map(),costObserved:0,restored:0,unconfirmed:0,duplicates:0,latestAt:0};
      groups.set(key,group);
    }
    group.records++;
    group.latestAt=Math.max(group.latestAt,row.completedAt??row.startedAt);
    if(row.readOnlyAfterRestore)group.restored++;
    if(row.status==="unconfirmed")group.unconfirmed++;
    if(row.contributesToTotals===false){group.duplicates++;continue;}
    if (row.tokens) {
      group.observed++;
      for(const [name] of counters){
        const value=row.tokens[name];
        if(value==null)continue;
        const metric=group.metrics[name];
        metric.observed++; metric.value+=value;
        metric.overflow ||= !Number.isSafeInteger(metric.value);
      }
    }
    if(row.cost){
      group.costObserved++;
      group.costs.set(row.cost.currency,addCost(group.costs.get(row.cost.currency),row.cost.amount,row.completedAt??row.startedAt));
    }
  }
  return [...groups.values()].sort((a,b)=>b.latestAt-a.latestAt || a.key.localeCompare(b.key));
}
const coverageText = (source:string) => source==="persisted_runtime_records"?"按已保存运行记录统计"
  :source==="persisted_request_receipts"?"按已保存的独立请求回执统计"
  :source==="persisted_and_current_page_receipts"?"按已保存及当前页面回执统计"
  :source==="current_runtime_records_usage_not_provided"?"当前运行可查看，用量未提供":"独立用量未提供";
export function PersonalUsageSettings() {
  const [view,setView]=useState<UsageView|null>(null);
  const [error,setError]=useState("");
  const [loading,setLoading]=useState(true);
  const [revision,setRevision]=useState(0);
  const [purpose,setPurpose]=useState<"all"|Purpose>("all");
  useEffect(()=>{
    let current=true;
    setLoading(true); setError(""); setView(null);
    void invoke<UsageView>("ai_usage_view").then(value=>{if(current)setView(checkedView(value));})
      .catch(reason=>{if(current)setError(aiErrorMessage(reason));}).finally(()=>{if(current)setLoading(false);});
    return ()=>{current=false;};
  },[revision]);
  const groups=useMemo(()=>groupUsage((view?.rows??[]).filter(row=>purpose==="all" || row.purpose===purpose)),[view,purpose]);
  const number=(metric:Metric)=>metric.overflow?"超出精确统计范围":metric.observed?metric.value.toLocaleString():"未提供";
  return <section className="personal-usage-page" aria-label="使用量">
    <header><h1>使用量</h1><button type="button" disabled={loading} onClick={()=>setRevision(value=>value+1)}>刷新</button></header>
    <div className="personal-usage-filter"><label htmlFor="usage-purpose">用途</label><Select id="usage-purpose" ariaLabel="用途" value={purpose} onChange={value=>setPurpose(value as "all"|Purpose)} options={[{value:"all",label:"全部"},...Object.entries(purposes).map(([value,label])=>({value,label}))]}/></div>
    {loading?<p role="status">正在读取…</p>:null}
    {error?<p role="alert">{error}</p>:null}
    {view?<>
      <p className="personal-usage-note">仅统计已提供的用量；缺失值不计为零。费用按上游明示币种分别合计。</p>
      <div className="personal-usage-table"><table><thead><tr><th>用途与模型</th><th>连接</th>{counters.map(([key,label])=><th key={key}>{label}</th>)}<th>记录覆盖</th><th>费用</th></tr></thead>
        <tbody>{groups.map(group=><tr key={group.key}><td>{purposes[group.purpose]}<small>{group.modelId??"模型未提供"}</small>
          {group.restored?<small>恢复历史：{group.restored} 条，只读</small>:null}
          {group.unconfirmed?<small>结果未确认：{group.unconfirmed} 条</small>:null}
          {group.duplicates?<small>重复回执：{group.duplicates} 条，未重复计数</small>:null}</td>
          <td><code>{group.connectionId??"未提供"}</code></td>
          {counters.map(([key,label])=><td key={key} aria-label={label}><span>{number(group.metrics[key])}</span>
            <small>{group.metrics[key].observed} / {group.records-group.duplicates} 条提供</small></td>)}
          <td>{group.observed} / {group.records-group.duplicates}
            {group.duplicates?<small>计入 {group.records-group.duplicates} / {group.records} 条</small>:null}</td><td aria-label="费用">
            {[...group.costs.entries()].sort(([a],[b])=>a.localeCompare(b)).map(([currency,total])=><div key={currency}>
              <span>{currency} {costText(total)}</span><small>{total.observed} / {group.records-group.duplicates} 条提供</small>
              <small>最近记录：{timeText(total.latestAt)}</small></div>)}
            {group.costObserved<group.records-group.duplicates?<small>{group.records-group.duplicates-group.costObserved} 条未提供</small>:null}
            {!group.costObserved?<span>未提供</span>:<small>来源：上游响应</small>}
          </td></tr>)}</tbody></table></div>
      {!groups.length?<p>此范围没有已记录的运行。</p>:null}
      <dl className="personal-usage-coverage">{view.coverage.filter(item=>purpose==="all" || purpose===item.purpose).map(item=><div key={item.purpose}><dt>{purposes[item.purpose]}</dt>
        <dd>{coverageText(item.source)}</dd></div>)}</dl>
      <p className="personal-usage-note">会话整理与自动审核分别记录。当前模型选择不改变历史统计；结果未确认的请求不会自动重发。</p>
    </>:null}
  </section>;
}
