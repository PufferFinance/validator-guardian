use anyhow::Result;
use atakit_portal_client::{HashFunction, PortalClient, PublicIdentity};
use log::info;
use serde_derive::{Deserialize, Serialize};
use std::time::Duration;

/// Attestation evidence from the atakit portal.
///
/// In the atakit/TDX model, attestation is workload-level (registered on-chain
/// via `SessionRegistry`) rather than per-request. The atakit portal signs each
/// payload with the session key, and clients verify the signature on-chain via
/// `SessionRegistry.verifySessionSignature()`.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
// GuardianModule checks owner_public_key against the registered session owner.
pub struct AttestationEvidence {
    pub session_id: String,
    pub signature: String,
    pub session_public_key: PublicIdentity,
    pub owner_public_key: PublicIdentity,
}

impl AttestationEvidence {
    /// Create new attestation evidence by signing `data` with the CVM session key.
    ///
    /// `data` is the payload to commit to (e.g. a public key). The atakit portal
    /// signs it and returns the session ID, signature, and session public key.
    /// Owner identity is recovered from evidence bound to the same session;
    /// unavailable authorization or a session change returns an error.
    pub async fn new(data: &[u8]) -> Result<Self> {
        if std::env::var("CVM_AGENT_STUB").unwrap_or_default() == "true" {
            info!("CVM_AGENT_STUB=true, returning stub attestation evidence");
            return Ok(AttestationEvidence::default());
        }
        info!("Requesting atakit portal attestation signature");
        let client = PortalClient::default().with_timeout(Duration::from_secs(15));
        let resp = client
            .clone()
            .with_response_limit(64 * 1024)
            .sign_message_when_ready(data, HashFunction::Keccak256, None)
            .await?;
        let owner = client
            .owner_public_key_for_session(resp.session_id, &resp.session_pubkey, None)
            .await?;
        Ok(AttestationEvidence {
            owner_public_key: PublicIdentity {
                type_id: owner.type_id,
                key: owner.key,
            },
            session_id: resp.session_id.to_string(),
            signature: resp.signature.to_string(),
            session_public_key: PublicIdentity {
                type_id: resp.session_pubkey.type_id,
                key: resp.session_pubkey.key,
            },
        })
    }
}
