//! CVM Agent client for communicating via Unix socket.
//!
//! Replaces SGX EPID remote attestation with session-based signing
//! through the CVM agent running inside the TDX Confidential VM.
//!
//! The CVM agent handles TDX DCAP attestation, TPM operations, and
//! SessionRegistry interaction. The application only needs to call
//! `/sign-message` to get a session-attested signature.

use alloy::primitives::{Bytes, B256};
use anyhow::Result;
use automata_cvm_agent::{client::CvmAgent, PublicIdentity};
use serde::{Deserialize, Serialize};

/// Default path to the CVM agent Unix socket
pub const CVM_AGENT_SOCKET_PATH: &str = "/app/cvm-agent.sock";

#[derive(Clone, Default, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionProof {
    pub session_id: B256,
    pub signature: Bytes,
    pub session_key: PublicIdentity,
    pub owner_key: PublicIdentity,
}

/// Get the appropriate attestation signer.
///
/// Returns a `SignMessageResponse` by signing the given message payload.
/// In a TDX CVM environment, this calls the real CVM agent.
/// In local development, this returns a mock proof.
pub async fn sign_with_session(message: &[u8]) -> Result<SessionProof> {
    // Check if the CVM agent socket exists
    let socket_path = std::env::var("CVM_AGENT_SOCKET_PATH");
    let socket_path = socket_path.as_deref().unwrap_or(CVM_AGENT_SOCKET_PATH);
    let client = CvmAgent::new(socket_path);
    let result = client.sign_message(message).await?;

    Ok(SessionProof {
        signature: result.signature,
        session_id: result.session_id,
        session_key: result.session_key_public,
        owner_key: result.owner_key_public,
    })
}
