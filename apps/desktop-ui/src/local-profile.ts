import { invoke } from './host';

export interface LocalProfileStatus { profileId: string; displayName: string; schemaVersion: number; ready: boolean; }

export async function loadLocalProfile(): Promise<LocalProfileStatus> {
  const profile = await invoke<LocalProfileStatus>('local_profile_status');
  if (!profile || profile.ready !== true || profile.schemaVersion !== 2 || typeof profile.profileId !== "string" || !/^[a-f0-9]{32}$/.test(profile.profileId) || typeof profile.displayName !== "string" || !profile.displayName.trim()) {
    throw new Error('本地资料格式不受支持，请查看诊断。');
  }
  return profile;
}
