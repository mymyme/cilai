import { useEffect, useRef, useState } from "react";
import { invoke } from "./host";
import { aiErrorMessage } from "./ai-runtime";
import { auditMoment } from "./datetime";
import type { PersonalAutomationFinishResult, PersonalAutomationTriggerItem, PersonalAutomationTriggerOutcome, PersonalAutomationTriggerPage } from "./generated/protocol";

const labels:Record<PersonalAutomationTriggerOutcome,string>={admitted:"已受理",skipped:"已跳过",completed:"已完成",failed:"失败",cancelled:"已取消",outcome_unconfirmed:"结果未确认"};
const reasons:Record<string,string>={
  automation_previous_run_active:"上次任务尚未结束，本次已跳过。",
  automation_missed_during_sleep:"睡眠期间错过计划，未补跑。",
  automation_power_epoch_unconfirmed:"无法核对睡眠状态，任务未继续。",
  automation_wake_boundary_unconfirmed:"无法核对唤醒边界，任务未继续。",
  automation_run_failed:"任务失败，请查看运行详情。",
  automation_interrupted_outcome_unconfirmed:"已核对退出，原结果仍未确认。",
};
const integer=(value:unknown):value is number=>typeof value==="number"&&Number.isSafeInteger(value)&&value>=0;
const identifier=(value:unknown):value is string=>typeof value==="string"&&value.length>0&&value.length<=160;
const optionalId=(value:unknown)=>value===null||identifier(value);
function checkedPage(value:PersonalAutomationTriggerPage,ruleId:string,before:number|null):PersonalAutomationTriggerPage {
  const invalid=()=>new Error("触发记录无法核对，请刷新。");
  if(!value||!Array.isArray(value.items)||value.items.length>20||!(value.nextBeforeSequence===null||integer(value.nextBeforeSequence)&&value.nextBeforeSequence>0))throw invalid();
  let previous=before??Number.POSITIVE_INFINITY;
  const ids=new Set<string>();
  for(const item of value.items){
    const r=item?.record;
    if(!item||!integer(item.sequence)||item.sequence===0||item.sequence>=previous||!r||!identifier(r.triggerId)||ids.has(r.triggerId)
      ||r.ruleId!==ruleId||!identifier(r.profileId)||!optionalId(r.claimId)||!optionalId(r.runId)
      ||(r.claimId===null?item.claimReleased!==null:typeof item.claimReleased!=="boolean")||!integer(r.ruleVersion)||r.ruleVersion>4_294_967_295
      ||!integer(r.createdAt)||!(r.finishedAt===null||integer(r.finishedAt))||!(r.scheduledDueAt===null||integer(r.scheduledDueAt))
      ||!Object.hasOwn(labels,r.outcome)||!(r.reasonCode===null||typeof r.reasonCode==="string"&&r.reasonCode.length<=200)
      ||typeof r.readOnlyAfterRestore!=="boolean")throw invalid();
    previous=item.sequence;ids.add(r.triggerId);
  }
  if(value.nextBeforeSequence!==null&&(!value.items.length||value.nextBeforeSequence>previous||before!==null&&value.nextBeforeSequence>=before))throw invalid();
  return value;
}

export function AutomationTriggerHistory(props:{ruleId:string;refreshToken:string;availableRunIds:string[];onOpenRun:(runId:string)=>void;onReconciled?:()=>void|Promise<void>}) {
  const [items,setItems]=useState<PersonalAutomationTriggerItem[]>([]);
  const [next,setNext]=useState<number|null>(null);
  const [revision,setRevision]=useState(0);
  const [loading,setLoading]=useState(true);
  const [error,setError]=useState("");
  const [notice,setNotice]=useState("");
  const [checking,setChecking]=useState<string|null>(null);
  const generation=useRef(0);
  const checkInFlight=useRef(false);
  const reading=useRef<number|null>(null);
  const read=async(before:number|null,epoch:number)=>{
    if(reading.current===epoch)return;
    reading.current=epoch;setLoading(true);setError("");
    try{
      const response=checkedPage(await invoke<PersonalAutomationTriggerPage>("ai_automation_trigger_history",{request:{ruleId:props.ruleId,limit:20,beforeSequence:before}}),props.ruleId,before);
      if(epoch!==generation.current)return;
      if(before!==null&&response.items.some(item=>items.some(known=>known.record.triggerId===item.record.triggerId)))throw new Error("触发记录已变化，请刷新。");
      setItems(current=>before===null?response.items:[...current,...response.items]);setNext(response.nextBeforeSequence);
    }catch(reason){if(epoch===generation.current)setError(aiErrorMessage(reason));}
    finally{if(reading.current===epoch)reading.current=null;if(epoch===generation.current)setLoading(false);}
  };
  useEffect(()=>{
    const epoch=++generation.current;
    setItems([]);setNext(null);setNotice("");
    void read(null,epoch);
    return()=>{generation.current++;};
  },[props.ruleId,props.refreshToken,revision]);
  const reconcile=async(item:PersonalAutomationTriggerItem)=>{
    if(checkInFlight.current||item.record.readOnlyAfterRestore||!item.record.claimId)return;
    checkInFlight.current=true;setChecking(item.record.triggerId);setError("");setNotice("");
    const epoch=generation.current;
    try{
      const value=await invoke<PersonalAutomationFinishResult>("ai_automation_claim_reconcile",{request:{claimId:item.record.claimId}});
      if(epoch!==generation.current)return;
      if(!value?.claim?.released||value.claim.claimId!==item.record.claimId||value.claim.profileId!==item.record.profileId||value.trigger?.triggerId!==item.record.triggerId||value.trigger.outcome!=="outcome_unconfirmed")throw new Error("结束状态未确认，占用保持。");
      const wrapper=value.ruleRecord?.value as {value?:{automationRuleId?:unknown;enabled?:unknown;nextRunAt?:unknown}}|undefined;
      if(value.ruleRecord&&(value.ruleRecord.id!==`automationRules:${props.ruleId}`||wrapper?.value?.automationRuleId!==props.ruleId||typeof wrapper.value.enabled!=="boolean"))throw new Error("占用已解除，规则状态无法核对，请刷新。");
      const paused=wrapper?.value?.enabled===false&&wrapper.value.nextRunAt===null;
      setNotice(paused?"已核对退出，规则已暂停。原结果仍未确认。":"占用已解除，原结果仍未确认。");
      await read(null,epoch);
      await props.onReconciled?.();
    }catch(reason){if(epoch===generation.current)setError(aiErrorMessage(reason));}
    finally{checkInFlight.current=false;setChecking(null);}
  };
  return <section className="automation-trigger-history" aria-label="触发记录">
    <header><h3>触发记录</h3><button type="button" aria-label="刷新触发记录" disabled={loading||checking!==null} onClick={()=>setRevision(value=>value+1)}>刷新</button></header>
    {loading?<p role="status">正在读取…</p>:null}
    {error?<p role="alert">{error}</p>:null}
    {notice?<p role="status">{notice}</p>:null}
    <ol>{items.map(item=>{
      const r=item.record;
      return <li key={r.triggerId}>
        <div><strong>{labels[r.outcome]}</strong><span>规则 v{r.ruleVersion} · {auditMoment(r.createdAt)}</span></div>
        {r.reasonCode?<p>{reasons[r.reasonCode]??"原因无法识别，请查看诊断。"}</p>:null}
        {r.readOnlyAfterRestore?<small>恢复历史，只读</small>:null}
        {item.claimReleased!==null?<small>{r.readOnlyAfterRestore?"历史占用：":""}{item.claimReleased?"占用已解除":"占用保持"}</small>:null}
        {r.runId&&!props.availableRunIds.includes(r.runId)?<small>运行详情暂不可定位</small>:null}
        <footer>{r.runId&&props.availableRunIds.includes(r.runId)?<button type="button" onClick={()=>props.onOpenRun(r.runId!)}>运行详情</button>:null}
          {!r.readOnlyAfterRestore&&r.claimId&&item.claimReleased===false&&["admitted","outcome_unconfirmed"].includes(r.outcome)?<button type="button" disabled={loading||checking!==null} onClick={()=>void reconcile(item)}>{checking===r.triggerId?"正在核对…":"核对结束状态"}</button>:null}</footer>
      </li>;
    })}</ol>
    {!loading&&!error&&!items.length?<p>暂无触发记录。</p>:null}
    {next!==null?<button type="button" disabled={loading||checking!==null} onClick={()=>void read(next,generation.current)}>更多</button>:null}
  </section>;
}
