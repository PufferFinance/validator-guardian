use atakit_portal_client::PortalClient;
#[tokio::test]
#[ignore = "requires a running emulator; set EMULATOR_SOCKET and EMULATOR_OWNER_FINGERPRINT"]
async fn current_client_recovers_owner_and_validates_evidence_binding() {
    let socket = std::env::var("EMULATOR_SOCKET").unwrap();
    let owner = std::env::var("EMULATOR_OWNER_FINGERPRINT")
        .unwrap()
        .parse()
        .unwrap();
    let client = PortalClient::new(socket);
    let _status = client.status().await.unwrap();
    let key = client.owner_public_key(owner, [0x37; 32]).await.unwrap();
    assert_eq!(key.fingerprint, owner);
    assert_eq!(key.type_id, 3);
    assert_eq!(key.key.len(), 65);
    let bundle = client.evidence_bundle([0x48; 32]).await.unwrap();
    bundle.validate_binding([0x48; 32]).unwrap();
    assert_eq!(
        bundle.claims().unwrap().binding.mode,
        atakit_portal_client::wire::BindingMode::Chain
    );
    #[derive(serde::Deserialize)]
    struct EmulatorEvidence {
        emulated: bool,
    }
    let evidence: EmulatorEvidence =
        serde_json::from_value(bundle.evidence_bundle.clone()).unwrap();
    assert!(evidence.emulated);
}
