use super::{ClientError, EvidenceBundle, EvidenceClaims, PublicKey};
use alloy::primitives::{B256, Signature, keccak256};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

const BINDING_DOMAIN: &str = "ATAKIT_PORTAL_SESSION_REQUEST_BINDING_EVIDENCE_BUNDLE_V1";
impl EvidenceBundle {
    /// Reads typed identity fields without changing the signed evidence document.
    pub fn claims(&self) -> Result<EvidenceClaims, ClientError> {
        Ok(serde_json::from_value(self.evidence_bundle.clone())?)
    }

    /// Checks that this evidence matches the request's challenge and binding signature.
    ///
    /// Pass the same fresh 32-byte challenge used to request the evidence.
    /// This detects mismatched or altered responses but does not establish TEE trust
    /// or verify on-chain session status.
    pub fn validate_binding(&self, challenge: [u8; 32]) -> Result<(), ClientError> {
        if self.request_binding.challenge != URL_SAFE_NO_PAD.encode(challenge) {
            return Err(ClientError::InvalidResponse(
                "evidence challenge mismatch".into(),
            ));
        }
        let key = self.claims()?.session_key;
        let key = PublicKey {
            type_id: key.type_id,
            key: key.bytes.parse().map_err(|e: alloy::hex::FromHexError| {
                ClientError::InvalidResponse(e.to_string())
            })?,
            fingerprint: parse_fingerprint(&key.fingerprint)?,
        };
        key.validate()
            .map_err(|e| ClientError::InvalidSignature(e.to_string()))?;
        let canonical = serde_json_canonicalizer::to_vec(&self.evidence_bundle)?;
        let mut payload = Vec::with_capacity(96);
        payload.extend_from_slice(keccak256(BINDING_DOMAIN).as_slice());
        payload.extend_from_slice(&challenge);
        payload.extend_from_slice(&Sha256::digest(canonical));
        let digest = B256::from_slice(&Sha256::digest(payload));
        let signature = alloy::hex::decode(&self.request_binding.signature)
            .map_err(|e| ClientError::InvalidSignature(e.to_string()))?;
        let recovered = recover_key(&signature, &digest)?;
        if recovered.as_slice() != key.key.as_ref() {
            return Err(ClientError::InvalidSignature(
                "evidence request signature does not match its session key".into(),
            ));
        }
        Ok(())
    }

    /// Recovers the owner public key and checks a trusted expected fingerprint.
    ///
    /// Validate the evidence binding before using this on a manually obtained bundle.
    /// The expected fingerprint must come from an independently trusted source.
    /// Does not check authorization expiry or on-chain session status.
    pub fn owner_public_key(&self, expected_owner: B256) -> Result<PublicKey, ClientError> {
        let key = self.recover_owner_public_key()?;
        if key.fingerprint != expected_owner {
            return Err(ClientError::InvalidSignature(
                "owner fingerprint differs from the trusted expected owner".into(),
            ));
        }
        Ok(key)
    }

    /// Recovers the owner public key advertised by this evidence.
    ///
    /// Requires owner authorization and checks the recovered key's fingerprint.
    /// Validate the evidence binding first. Use this for identity inspection;
    /// the advertised fingerprint alone is not a trusted identity.
    pub fn recover_owner_public_key(&self) -> Result<PublicKey, ClientError> {
        let claims = self.claims()?;
        if claims.binding.mode != crate::wire::BindingMode::Chain {
            return Err(ClientError::InvalidResponse(
                "owner authorization unavailable in local-bound evidence".into(),
            ));
        }
        let owner = claims.owner;
        let auth = owner.contract_authorization.ok_or_else(|| {
            ClientError::InvalidResponse("owner authorization unavailable".into())
        })?;
        let signature = alloy::hex::decode(&auth.signature)
            .map_err(|e| ClientError::InvalidSignature(e.to_string()))?;
        let key = recover_key(&signature, &parse_fingerprint(&auth.payload)?)?;
        let public_key = PublicKey {
            type_id: 3,
            key: key.into(),
            fingerprint: parse_fingerprint(&owner.fingerprint)?,
        };
        public_key
            .validate()
            .map_err(|e| ClientError::InvalidSignature(e.to_string()))?;
        Ok(public_key)
    }
}
/// Recovers a secp256k1 key from a 65-byte recoverable signature and a prehash.
///
/// Returns uncompressed SEC1 bytes with the `0x04` prefix. Signature parsing
/// or recovery failures produce a signature error.
fn recover_key(signature: &[u8], digest: &B256) -> Result<Vec<u8>, ClientError> {
    let key = Signature::try_from(signature)
        .and_then(|signature| signature.recover_from_prehash(digest))
        .map_err(|e| ClientError::InvalidSignature(e.to_string()))?;
    Ok(key.to_encoded_point(false).as_bytes().to_vec())
}

/// Parses a protocol hex field as a 32-byte value.
fn parse_fingerprint(value: &str) -> Result<B256, ClientError> {
    value
        .parse::<B256>()
        .map_err(|e| ClientError::InvalidResponse(e.to_string()))
}
