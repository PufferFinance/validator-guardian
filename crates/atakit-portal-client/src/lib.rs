#![doc = include_str!("../README.md")]
//! Workload-facing Portal client. Uses HTTP over the service Unix socket.
mod client;
mod evidence;
mod types;
pub mod wire;
pub use client::{ClientError, PortalClient, Reply};
pub use types::*;
pub use wire::BabyCreateRequest;

use alloy::{
    primitives::{Address, B256, Bytes as HexBytes, Signature, U256, keccak256},
    sol_types::SolValue,
};
use serde::{Deserialize, Serialize};

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub const SIGN_DOMAIN: &[u8] = b"ATAKIT_SESSION_SIGN_V1";

/// Compute the domain-separated digest expected by Portal signature verifiers.
/// Pass the original message bytes, without adding the signing domain yourself.
pub fn signing_digest(message: &[u8], hash_fn: HashFunction) -> B256 {
    match hash_fn {
        HashFunction::Keccak256 => keccak256([SIGN_DOMAIN, message].concat()),
        HashFunction::Sha256 => {
            use sha2::{Digest, Sha256};
            B256::from_slice(&Sha256::digest([SIGN_DOMAIN, message].concat()))
        }
    }
}

/// Public identity included in application responses and contract proofs.
#[derive(Clone, Default, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublicIdentity {
    #[serde(alias = "type_id")]
    pub type_id: u8,
    pub key: HexBytes,
}

/// Parsed public identity for local validation and contract calls.
#[derive(Debug, Serialize, Deserialize)]
pub struct PublicKey {
    pub type_id: u8,
    pub key: HexBytes,
    pub fingerprint: B256,
}

/// Parsed signing response for local validation and contract calls.
#[derive(Debug, Serialize, Deserialize)]
pub struct SignResponse {
    pub hash_fn: String,
    pub message_hash: B256,
    pub signature: HexBytes,
    pub session_id: B256,
    pub session_pubkey: PublicKey,
}

impl PublicKey {
    /// Checks that the public key has a supported format and matching fingerprint.
    ///
    /// Supports 65-byte uncompressed ES256K keys. This is a format and identity
    /// consistency check, not proof that the owner or session is trusted.
    pub fn validate(&self) -> Result<(), Error> {
        let key = self;
        if key.type_id != 3 || key.key.len() != 65 || key.key[0] != 4 {
            return Err("Portal returned an unsupported session public key".into());
        }
        let fingerprint = keccak256(
            (
                keccak256("KEY_RESOLVER_V1"),
                U256::from(key.type_id),
                key.key.clone(),
            )
                .abi_encode_params(),
        );
        if key.fingerprint != fingerprint {
            return Err("Portal session public-key fingerprint mismatch".into());
        }
        Ok(())
    }
}

impl SignResponse {
    /// Checks that this signature response matches the original message bytes.
    ///
    /// Validates the signing domain, message hash, key fingerprint, and signature.
    /// Does not verify an expected session ID, chain registration, or TEE trust.
    pub fn validate(&self, message: &[u8]) -> Result<(), Error> {
        let hash_fn = match self.hash_fn.as_str() {
            "keccak256" => HashFunction::Keccak256,
            "sha256" => HashFunction::Sha256,
            _ => return Err("unsupported signing hash function".into()),
        };
        let expected_hash = signing_digest(message, hash_fn);
        if self.message_hash != expected_hash {
            return Err("Portal returned a different signing hash or hash function".into());
        }
        let key = &self.session_pubkey;
        key.validate()?;
        let signature = Signature::try_from(self.signature.as_ref())?;
        let recovered = signature.recover_address_from_prehash(&expected_hash)?;
        if recovered != Address::from_raw_public_key(&key.key[1..]) {
            return Err("Portal signature does not match its session public key".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::signers::{SignerSync, local::PrivateKeySigner};

    fn signed(message: &[u8]) -> SignResponse {
        let signer: PrivateKeySigner =
            "0000000000000000000000000000000000000000000000000000000000000001"
                .parse()
                .unwrap();
        let key = HexBytes::from(
            signer
                .credential()
                .verifying_key()
                .to_encoded_point(false)
                .as_bytes()
                .to_vec(),
        );
        let hash = keccak256([b"ATAKIT_SESSION_SIGN_V1".as_slice(), message].concat());
        SignResponse {
            hash_fn: "keccak256".into(),
            message_hash: hash,
            signature: signer
                .sign_hash_sync(&hash)
                .unwrap()
                .as_bytes()
                .to_vec()
                .into(),
            session_id: B256::repeat_byte(1),
            session_pubkey: PublicKey {
                type_id: 3,
                fingerprint: keccak256(
                    (keccak256("KEY_RESOLVER_V1"), U256::from(3), key.clone()).abi_encode_params(),
                ),
                key,
            },
        }
    }

    #[test]
    fn validates_current_portal_response_and_rejects_tampered_message() {
        let response = signed(b"hello");
        let json = serde_json::to_string(&response).unwrap();
        let decoded: SignResponse = serde_json::from_str(&json).unwrap();
        decoded.validate(b"hello").unwrap();
        assert!(decoded.validate(b"other message").is_err());
    }

    #[test]
    fn validates_independent_foundry_signing_vector() {
        let signed: SignResponse =
            serde_json::from_str(include_str!("../tests/sign-response.json")).unwrap();
        signed.validate(b"hello").unwrap();
    }

    #[test]
    fn rejects_legacy_undomained_hash() {
        let mut response = signed(b"hello");
        response.message_hash = keccak256(b"hello");
        assert!(response.validate(b"hello").is_err());
    }

    #[test]
    fn rejects_wrong_fingerprint_key_and_signature() {
        let mut response = signed(b"hello");
        response.session_pubkey.fingerprint = B256::ZERO;
        assert!(response.validate(b"hello").is_err());
        let mut response = signed(b"hello");
        response.signature = signed(b"other").signature;
        assert!(response.validate(b"hello").is_err());
        let mut response = signed(b"hello");
        response.session_pubkey.key = HexBytes::new();
        assert!(response.validate(b"hello").is_err());
    }
}

impl TryFrom<wire::SessionPubkeyResponse> for PublicKey {
    type Error = Error;
    /// Parses the protocol's hex-encoded public identity.
    fn try_from(value: wire::SessionPubkeyResponse) -> Result<Self, Error> {
        Ok(Self {
            type_id: value.type_id,
            key: value.key.parse()?,
            fingerprint: value.fingerprint.parse()?,
        })
    }
}

impl TryFrom<wire::SignMessageResponse> for SignResponse {
    type Error = Error;
    /// Parses a protocol response; signature validation is a separate step.
    fn try_from(value: wire::SignMessageResponse) -> Result<Self, Error> {
        Ok(Self {
            hash_fn: value.hash_fn,
            message_hash: value.message_hash.parse()?,
            signature: value.signature.parse()?,
            session_id: value.session_id.parse()?,
            session_pubkey: value.session_pubkey.try_into()?,
        })
    }
}

/// Message digest algorithms supported by Portal signing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashFunction {
    Keccak256,
    Sha256,
}
impl HashFunction {
    /// Returns the algorithm name accepted by Portal.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Keccak256 => "keccak256",
            Self::Sha256 => "sha256",
        }
    }
}
