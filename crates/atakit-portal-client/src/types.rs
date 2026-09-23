//! Workload protocol types synchronized with atakit-portal.
//! See PROTOCOL.md for the source revision and client-specific representations.
use crate::wire::{SessionBinding, SessionOwner, SessionPublicKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortalStatus {
    pub init_schema_version: u32,
    pub state: String,
    #[serde(default)]
    pub base_image_id: Option<String>,
    #[serde(default)]
    pub base_image_ref: Option<String>,
    #[serde(default)]
    pub workload_id: Option<String>,
    #[serde(default)]
    pub workload_ref: Option<String>,
    #[serde(default)]
    pub gas_wallet_address: Option<String>,
    pub started_at: String,
    #[serde(default)]
    pub detail: Option<String>,
    pub platform: PlatformStatus,
    pub chain: ChainStatus,
    #[serde(default)]
    pub prover: Option<ProverStatus>,
}

/// Portal evidence response. The complete JSON value is covered by its binding signature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceBundle {
    pub evidence_bundle: Value,
    pub request_binding: RequestBinding,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestBinding {
    pub challenge: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BabyImage {
    pub image_id: String,
    pub slot: String,
    pub uploaded_by: String,
    pub retention: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BabyInstance {
    pub instance_id: String,
    pub slot: String,
    pub image_id: String,
    pub container_id: String,
    pub container_name: String,
    pub parent_service: String,
    pub uid_slot: u64,
    #[serde(default)]
    pub cap_add: Vec<String>,
    pub status: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BabyImageUpload {
    pub image_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BabyImages {
    pub images: Vec<BabyImage>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BabyCreated {
    pub instance: BabyInstance,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BabyContainers {
    pub images: Vec<BabyImage>,
    pub instances: Vec<BabyInstance>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BabyLogs {
    pub logs: String,
}

/// Fixed identity fields read from a signed evidence document.
#[derive(Debug, Clone, Deserialize)]
pub struct EvidenceClaims {
    pub session_id: Option<String>,
    pub binding: SessionBinding,
    pub session_key: SessionPublicKey,
    pub owner: SessionOwner,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainStatus {
    pub registration: String,
    pub tee_backend: String,
    pub status: String,
    #[serde(default)]
    pub registry_address: Option<String>,
    #[serde(default)]
    pub chain_id: Option<u64>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub tx_hash: Option<String>,
    #[serde(default)]
    pub block_number: Option<u64>,
    #[serde(default)]
    pub submitted_at: Option<String>,
    #[serde(default)]
    pub verified_at: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudDetection {
    pub dmi: DmiObservation,
    pub metadata: MetadataObservation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudProvenance {
    pub source: CloudSource,
    pub detection: CloudDetection,
    /// Exact accepted `platform.declared` input, when one was supplied.
    pub user_provided: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DmiObservation {
    pub sys_vendor: Option<String>,
    pub product_name: Option<String>,
    pub bios_vendor: Option<String>,
    pub detected_cloud: CloudProvider,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataObservation {
    pub gcp: MetadataProbe,
    pub azure: MetadataProbe,
    pub aws: MetadataProbe,
    /// Provider inferred only from successful metadata probes. `Unknown`
    /// includes both no match and ambiguous multiple matches; `conflict`
    /// distinguishes those cases.
    pub detected_cloud: CloudProvider,
    pub conflict: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataProbe {
    pub attempted: bool,
    pub matched: bool,
    pub http_status: Option<u16>,
    pub response_headers: BTreeMap<String, String>,
    pub response_body: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformStatus {
    pub build_profile: String,
    pub cloud_type: String,
    pub cloud_provenance: CloudProvenance,
    pub attestation_mode: String,
    pub tee_type: String,
    pub machine_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProverStatus {
    pub backend: String,
    pub execution: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudProvider {
    Gcp,
    Azure,
    Aws,
    Qemu,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudSource {
    Dmi,
    Metadata,
    UserProvided,
    Unresolved,
}
