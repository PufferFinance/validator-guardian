//! Portal protocol types, without client-side key or hash conversions.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionPublicKey {
    pub type_id: u8,
    pub bytes: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionOwner {
    pub fingerprint: String,
    /// Optional on-chain transaction projection. It remains request-bound as
    /// part of the bundle JSON but is not an offline session-validity input.
    pub contract_authorization: Option<SessionContractAuthorization>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionContractAuthorization {
    pub op_expires_at: u64,
    pub payload: String,
    pub signature: String,
    #[serde(default)]
    pub calldata_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionBinding {
    pub mode: BindingMode,
    pub chain_id: u64,
    pub registry: String,
    pub owner_nonce: String,
    pub qualifying_data: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingMode {
    Chain,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignMessageRequest {
    pub message: String,
    #[serde(default)]
    pub hash_fn: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_expected_session_id",
        serialize_with = "serialize_expected_session_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub expected_session_id: Option<[u8; 32]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignMessageResponse {
    pub hash_fn: String,
    pub message_hash: String,
    pub signature: String,
    pub session_id: String,
    pub session_pubkey: SessionPubkeyResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionPubkeyResponse {
    pub type_id: u8,
    pub key: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BabyImageRemoveRequest {
    pub image_id: String,
    pub slot: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BabyInstanceRequest {
    pub instance_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BabyCreateRequest {
    pub slot: String,
    pub image_id: Option<String>,
    pub instance_id: Option<String>,
    #[serde(default, rename = "cap-add", alias = "cap_add")]
    pub cap_add: Vec<String>,
}

/// Validates Portal's lowercase, 32-byte session ID format.
fn decode_canonical_session_id(value: &str) -> Result<[u8; 32], String> {
    const ERROR: &str = "expected_session_id must be canonical 0x-prefixed 32-byte lowercase hex";
    if value.len() != 66 || !value.starts_with("0x") {
        return Err(ERROR.to_string());
    }
    let encoded = &value[2..];
    if !encoded
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ERROR.to_string());
    }
    let bytes = alloy::hex::decode(encoded).map_err(|_| ERROR.to_string())?;
    bytes.try_into().map_err(|_| ERROR.to_string())
}

/// Reads a present session ID; explicit null is rejected by Portal.
fn deserialize_expected_session_id<'de, D>(deserializer: D) -> Result<Option<[u8; 32]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    decode_canonical_session_id(&value)
        .map(Some)
        .map_err(serde::de::Error::custom)
}

/// Encodes an expected session ID in Portal's canonical lowercase hex format.
fn serialize_expected_session_id<S: serde::Serializer>(
    value: &Option<[u8; 32]>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(id) => serializer.serialize_str(&alloy::hex::encode_prefixed(id)),
        None => serializer.serialize_none(),
    }
}
