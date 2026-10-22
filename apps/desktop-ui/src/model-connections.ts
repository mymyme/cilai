import { invoke } from "./host";
import type { ModelNetworkSecret, NetworkSave } from "./model-network-settings";
import type {
  ModelConnection, ModelConnectionChange, ModelConnectionConfig, ModelConnectionPreflight,
  ModelCredentialAction, ModelDescriptor, ModelPurposeBinding, ModelValidationRecord,
  ModelExecutionRecoverResult,
  ModelValidationPrepareParams,
} from "./generated/protocol";

export type ModelMetadataMutation<T> = { result: T; catalogRefreshError: string | null };
export const refreshLocalModelCatalog = () => invoke("ai_model_catalog_read", { reason: "retry" });

export const loadModelConnections = () => invoke<ModelConnection[]>("model_connection_list", { request: {} });
export const loadModelConnectionChanges = () => invoke<ModelConnectionChange[]>("model_connection_change_list", { request: {} });
export const loadConnectionModels = (connectionId: string) => invoke<ModelDescriptor[]>("model_descriptor_list", { request: { connectionId } });
export const loadModelPurposes = () => invoke<ModelPurposeBinding[]>("model_purpose_list", { request: {} });
export const loadConnectionValidations = (connectionId: string) => invoke<ModelValidationRecord[]>("model_validation_list", { request: { connectionId } });
export const refreshConnectionModels = (connectionId: string) => invoke<ModelDescriptor[]>("model_connection_catalog_refresh", { request: { connectionId } });
export const validateModelConnection = (request: ModelValidationPrepareParams) => invoke<ModelValidationRecord>("model_connection_validate", { request });

export function saveModelConnection(input: {
  operationId: string; connection: ModelConnection | null; config: ModelConnectionConfig;
  credentialAction: ModelCredentialAction; secret?: string;
  network?: NetworkSave;
}) {
  return invoke<ModelMetadataMutation<ModelConnection>>("model_connection_save", { request: {
    prepare: { operationId: input.operationId, connectionId: input.connection?.connectionId ?? null,
      expectedConfigRevision: input.connection?.configRevision ?? null, config: input.config,
      credentialAction: input.credentialAction, networkCredentialAction: input.network?.networkCredentialAction ?? "keep" }, secret: input.secret ?? null,
      networkSecret: input.network?.networkSecret ?? null,
  } });
}

export const resumeModelConnection = (operationId: string, secret?: string, networkSecret?: ModelNetworkSecret) =>
  invoke<ModelMetadataMutation<ModelConnection>>("model_connection_resume", { request: { operationId, secret: secret ?? null, networkSecret: networkSecret ?? null } });
export const abortModelConnection = (operationId: string) =>
  invoke<ModelConnectionChange>("model_connection_abort", { request: { operationId } });
export const cleanupModelCredential = (operationId: string) =>
  invoke<ModelConnectionChange>("model_connection_credential_cleanup", { request: { operationId } });
export const preflightModelConnection = (connectionId: string, action: "disable" | "delete" | "disconnect") =>
  invoke<ModelConnectionPreflight>("model_connection_preflight", { request: { connectionId, action } });
export const recoverModelExecutionLeases = (connectionId: string) =>
  invoke<ModelExecutionRecoverResult>("model_connection_recover_leases", { request: { connectionId } });
export const setModelConnectionEnabled = (connection: ModelConnection, enabled: boolean, impact?: ModelConnectionPreflight) =>
  invoke<ModelMetadataMutation<ModelConnection>>("model_connection_set_enabled", { request: { connectionId: connection.connectionId,
    expectedConfigRevision: connection.configRevision, enabled, impactDigest: impact?.impactDigest ?? null } });
export const deleteModelConnection = (impact: ModelConnectionPreflight) =>
  invoke<ModelMetadataMutation<ModelConnection>>("model_connection_delete", { request: { connectionId: impact.connectionId,
    expectedConfigRevision: impact.configRevision, impactDigest: impact.impactDigest } });
export const saveConnectionModel = (connectionId: string, modelId: string, displayName: string,
  details: { contextWindowTokens: number | null; inputModalities: string[]; defaultReasoningEffort: string | null; supportedReasoningEfforts: string[] }, expectedRevision: number | null) =>
  invoke<ModelMetadataMutation<ModelDescriptor>>("model_descriptor_save", { request: { connectionId, modelId, displayName, expectedRevision, ...details } });
export const saveModelPurpose = (binding: ModelPurposeBinding, expectedRevision: number | null) =>
  invoke<ModelMetadataMutation<ModelPurposeBinding>>("model_purpose_save", { request: { binding, expectedRevision } });
