import {invoke} from "./host";

export type PersonalNotificationPreferences = {
  enabled:boolean; completed:boolean; failed:boolean; actionRequired:boolean; sound:boolean; revision:number;
};
export type PersonalNotificationSettings = {
  profileId:string;
  preferences:PersonalNotificationPreferences;
  system:{supported:boolean;active:boolean;reasonCode:string|null;lastOutcome:string|null;profileId:string|null};
};
function checked(value:PersonalNotificationSettings,profileId?:string):PersonalNotificationSettings {
  if(!value||!/^[a-f0-9]{32}$/.test(value.profileId)||(profileId&&value.profileId!==profileId)
    ||!value.preferences||!Number.isSafeInteger(value.preferences.revision)||value.preferences.revision<0
    ||!["enabled","completed","failed","actionRequired","sound"].every(key=>typeof value.preferences[key as keyof PersonalNotificationPreferences]==="boolean")
    ||!value.system||typeof value.system.supported!=="boolean"||typeof value.system.active!=="boolean"||value.system.profileId!==value.profileId
    ||(value.system.reasonCode!==null&&typeof value.system.reasonCode!=="string"))throw new Error("personal_notification_response_invalid");
  return value;
}
export async function loadPersonalNotificationSettings(profileId?:string) {
  return checked(await invoke<PersonalNotificationSettings>("personal_notification_preferences_get",{}),profileId);
}
export async function savePersonalNotificationSettings(previous:PersonalNotificationSettings,preferences:PersonalNotificationPreferences) {
  return checked(await invoke<PersonalNotificationSettings>("personal_notification_preferences_save",{request:{
    profileId:previous.profileId,expectedRevision:previous.preferences.revision,
    enabled:preferences.enabled,completed:preferences.completed,failed:preferences.failed,
    actionRequired:preferences.actionRequired,sound:preferences.sound,
  }}),previous.profileId);
}
