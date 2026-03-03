//! Off-chain SessionRegistry verification via `eth_call`.
//!
//! Calls `SessionRegistry.verifySessionSignature()` as a view function
//! to verify that a CVM session signature is valid and the session is active.
//! No transaction is submitted — this is a read-only RPC call.

use alloy::primitives::{Address, B256, Bytes};
use alloy::providers::ProviderBuilder;
use alloy::sol;
use anyhow::{Context, Result};
use log::info;

sol! {
    #[sol(rpc)]
    interface ISessionRegistry {
        struct PublicIdentity {
            uint8 typeId;
            bytes key;
        }

        struct CVMSession {
            bytes32 sessionId;
            bytes32 owner;
            bytes32 akPubKeyFingerprint;
            bytes32 tpmSigningKeyFingerprint;
            bytes32 sessionKeyFingerprint;
            bytes32 baseImageId;
            bytes32 workloadId;
            bytes32 platformProfileId;
            bytes32 measurementVariantId;
            uint64 registeredAt;
            uint64 expiresAt;
            bool isActive;
        }

        function verifySessionSignature(
            bytes32 sessionId,
            PublicIdentity calldata sessionKey,
            bytes32 message,
            bytes calldata signature
        ) external view returns (bool valid);

        function getSession(bytes32 sessionId) external view returns (CVMSession memory session);

        function isSessionActive(bytes32 sessionId) external view returns (bool);
    }
}

/// Verify a session signature off-chain by calling `SessionRegistry.verifySessionSignature()`
/// via `eth_call`. Returns `true` if the session is active and the signature is valid.
pub async fn verify_session_signature(
    rpc_url: &str,
    registry_address: Address,
    session_id: B256,
    session_key: ISessionRegistry::PublicIdentity,
    message: B256,
    signature: Bytes,
) -> Result<bool> {
    info!(
        "Calling SessionRegistry.verifySessionSignature() at {} for session {}",
        registry_address, session_id
    );

    let provider = ProviderBuilder::new()
        .connect_http(rpc_url.parse().context("Invalid RPC URL")?);

    let registry = ISessionRegistry::new(registry_address, &provider);

    let result = registry
        .verifySessionSignature(session_id, session_key, message, signature)
        .call()
        .await
        .context("SessionRegistry.verifySessionSignature() call failed")?;

    Ok(result)
}

/// Fetch session details from `SessionRegistry.getSession()`.
pub async fn get_session(
    rpc_url: &str,
    registry_address: Address,
    session_id: B256,
) -> Result<ISessionRegistry::CVMSession> {
    info!(
        "Calling SessionRegistry.getSession() at {} for session {}",
        registry_address, session_id
    );

    let provider = ProviderBuilder::new()
        .connect_http(rpc_url.parse().context("Invalid RPC URL")?);

    let registry = ISessionRegistry::new(registry_address, &provider);

    let result = registry
        .getSession(session_id)
        .call()
        .await
        .context("SessionRegistry.getSession() call failed")?;

    Ok(result)
}
