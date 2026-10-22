import { invoke } from "./host";

export async function rpc<T>(method: string, params: object = {}): Promise<T> {
  return invoke<T>("app_server_request", { method, params });
}

export async function sha256(value: string): Promise<string> {
  const bytes = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return [...new Uint8Array(bytes)]
    .map((item) => item.toString(16).padStart(2, "0"))
    .join("");
}
