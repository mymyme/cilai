export const personalNotificationOpenEvent = "personal-notification-open";
export type PersonalNotificationOpen = {
  profileId:string; notificationId:string;
  target: {kind:"run";runId:string;conversationId:string}
    | {kind:"automation";automationRuleId:string}
    | {kind:"application";applicationId:string;sessionId:string;runId:string;creationProjectId:string;documentUrl:string};
};
const id=(value:unknown)=>typeof value==="string"&&value.length>0&&value.length<=200&&/^[A-Za-z0-9_.:-]+$/.test(value);
export function personalNotificationTarget(value:unknown,profileId:string):PersonalNotificationOpen["target"] {
  const payload=value as PersonalNotificationOpen;
  if(!payload||payload.profileId!==profileId||!/^[a-f0-9]{32}$/.test(payload.notificationId)||!payload.target)throw new Error("personal_notification_target_missing");
  const target=payload.target;
  if(target.kind==="run"&&id(target.runId)&&id(target.conversationId))return target;
  if(target.kind==="automation"&&id(target.automationRuleId))return target;
  if(target.kind==="application"&&id(target.applicationId)&&id(target.sessionId)&&id(target.runId)&&id(target.creationProjectId)
    &&typeof target.documentUrl==="string"&&/^app:\/\/personal-capability\/[a-f0-9-]{36}$/.test(target.documentUrl)&&target.documentUrl.length<=1024)return target;
  throw new Error("personal_notification_target_missing");
}
