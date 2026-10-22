type CredentialReference = { service: string } | null | undefined;

const workbenchServices = {
  model: "cn.cilai.personal.workbench.vault.model",
  "model-network": "cn.cilai.personal.workbench.vault.model-network",
  mcp: "cn.cilai.personal.workbench.vault.mcp",
} as const;

export function hasWorkbenchCredentialLocation(reference: CredentialReference, kind: keyof typeof workbenchServices): boolean {
  return reference?.service === workbenchServices[kind];
}
