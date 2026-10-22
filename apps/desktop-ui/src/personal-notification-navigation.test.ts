import {expect,it} from "vitest";
import {personalNotificationTarget} from "./personal-notification-navigation";
const profileId="1".repeat(32);
const envelope=(target:unknown)=>({profileId,notificationId:"a".repeat(32),target});
it("三种通知只接受当前资料的有界目标",()=>{
  for(const target of [
    {kind:"run",runId:"run_one",conversationId:"conversation-one"},
    {kind:"automation",automationRuleId:"automation-one"},
    {kind:"application",applicationId:"application-one",sessionId:"session-one",runId:"run-one",creationProjectId:"capability_one",documentUrl:"app://personal-capability/11111111-1111-4111-8111-111111111111"},
  ])expect(personalNotificationTarget(envelope(target),profileId)).toEqual(target);
});
it("跨资料、任意路径、脚本和失效文档不会变成页面导航",()=>{
  const target={kind:"application",applicationId:"app",sessionId:"session",runId:"run",creationProjectId:"project",documentUrl:"app://personal-capability/11111111-1111-4111-8111-111111111111"};
  for(const payload of [
    null,{...envelope(target),profileId:"2".repeat(32)},{...envelope(target),notificationId:"../../private"},
    envelope({...target,documentUrl:"https://outside.example"}),envelope({...target,documentUrl:"javascript:execute()"}),
    envelope({...target,documentUrl:"app://personal-capability/../private"}),envelope({kind:"run",runId:"../../private",conversationId:"conversation"}),
    envelope({kind:"automation",automationRuleId:"x".repeat(201)}),
  ])expect(()=>personalNotificationTarget(payload,profileId)).toThrow("personal_notification_target_missing");
});
