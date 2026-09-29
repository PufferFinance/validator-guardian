use alloy::sol_types::SolValue;
use puffersecuresigner::io::session_registry::ISessionRegistry;

#[test]
fn decodes_current_registry_session_without_legacy_owner_and_active_fields() {
    // getSession returns seven bytes32 fields, then registeredAt/sessionExpiresAt.
    // Distinct words catch accidental shifts of the workload ID and timestamps.
    let mut encoded = Vec::new();
    for value in 1u8..=9 {
        encoded.extend_from_slice(&[0; 31]);
        encoded.push(value);
    }
    let session = ISessionRegistry::CVMSession::abi_decode_params(&encoded).unwrap();
    assert_eq!(session.workloadId[31], 5);
    assert_eq!(session.registeredAt, 8);
}
