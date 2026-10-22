import {useEffect,useRef,useState} from "react";
import {loadPersonalNotificationSettings,savePersonalNotificationSettings} from "./personal-notifications";
import type {PersonalNotificationPreferences,PersonalNotificationSettings} from "./personal-notifications";
import "./personal-notification-settings.css";

function message(reason:unknown) {
  const tokens=new Set((reason instanceof Error?reason.message:String(reason)).match(/[a-z_]+/g)??[]);
  if(tokens.has("personal_notification_paused")||tokens.has("personal_data_maintenance_active"))return "资料维护中，请稍后刷新。";
  if(tokens.has("personal_notification_preferences_changed"))return "设置已变化，请刷新核对。";
  if(tokens.has("personal_notification_profile_changed"))return "当前资料已变化，请刷新。";
  return "结果尚未确认，请刷新核对。";
}
const fields=[{key:"completed",label:"完成"},{key:"failed",label:"失败"},{key:"actionRequired",label:"待确认"},{key:"sound",label:"声音"}] as const;
export function PersonalNotificationSettingsPanel({disabled=false,profileId}:{disabled?:boolean;profileId?:string}) {
  const [saved,setSaved]=useState<PersonalNotificationSettings|null>(null);
  const [draft,setDraft]=useState<PersonalNotificationPreferences|null>(null);
  const [busy,setBusy]=useState(false);
  const [unknown,setUnknown]=useState(false);
  const [error,setError]=useState("");
  const [notice,setNotice]=useState("");
  const generation=useRef(0);
  const accept=(result:PersonalNotificationSettings)=>{setSaved(result);setDraft({...result.preferences});setUnknown(false);};
  useEffect(()=>{
    const version=++generation.current;
    setSaved(null);setDraft(null);setError("");setNotice("");setUnknown(false);setBusy(true);
    void loadPersonalNotificationSettings(profileId).then(result=>{if(generation.current===version)accept(result);})
      .catch(reason=>{if(generation.current===version)setError(message(reason));})
      .finally(()=>{if(generation.current===version)setBusy(false);});
    return()=>{generation.current++;};
  },[profileId]);
  const refresh=async()=>{
    if(busy)return;
    const version=generation.current;setBusy(true);setError("");setNotice("");
    try{const result=await loadPersonalNotificationSettings(profileId);if(generation.current===version)accept(result);}
    catch(reason){if(generation.current===version)setError(message(reason));}
    finally{if(generation.current===version)setBusy(false);}
  };
  const save=async()=>{
    if(!saved||!draft||busy||unknown||disabled)return;
    const version=generation.current;setBusy(true);setError("");setNotice("");
    try{const result=await savePersonalNotificationSettings(saved,draft);if(generation.current===version){accept(result);setNotice("已保存");}}
    catch(reason){if(generation.current===version){setUnknown(true);setError(message(reason));}}
    finally{if(generation.current===version)setBusy(false);}
  };
  const locked=disabled||busy||unknown||!draft;
  const changed=saved&&draft&&JSON.stringify(saved.preferences)!==JSON.stringify(draft);
  return <section className="personal-notification-settings" aria-label="本机通知">
    <div className="personal-notification-title"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9M10 21h4"/></svg><h3>本机通知</h3></div>
    <label className="personal-notification-toggle"><span>允许通知</span><input type="checkbox" checked={draft?.enabled??false} disabled={locked} onChange={event=>setDraft(value=>value?{...value,enabled:event.target.checked}:value)}/></label>
    <div className="personal-notification-options">{fields.map(field=><label key={field.key}><input type="checkbox" checked={draft?.[field.key]??false} disabled={locked||!draft?.enabled} onChange={event=>setDraft(value=>value?{...value,[field.key]:event.target.checked}:value)}/>{field.label}</label>)}</div>
    {saved&&!saved.system.supported?<p role="status">当前环境不支持系统通知。</p>:null}
    {saved?.system.reasonCode==="personal_notification_paused"?<p role="status">资料维护中，通知已暂停。</p>:null}
    {saved?.system.reasonCode==="personal_notification_delivery_limit"?<p role="status">投递记录已达上限，已停止新通知。</p>:null}
    {saved?.system.lastOutcome==="failed"?<p role="status">系统投递失败，请检查系统通知权限。</p>:null}
    {saved?.system.lastOutcome==="unknown"?<p role="status">系统投递结果未确认，不会补发。</p>:null}
    <p className="personal-notification-note">仅显示任务状态。自动化完成通知还需开启该规则的“所有运行”。</p>
    <div className="personal-notification-actions"><button type="button" disabled={locked||!changed} onClick={()=>void save()}>{busy?"处理中…":"保存"}</button><button type="button" disabled={busy} onClick={()=>void refresh()}>刷新</button></div>
    {error?<p role="status">{error}</p>:null}{notice?<p role="status">{notice}</p>:null}
  </section>;
}
