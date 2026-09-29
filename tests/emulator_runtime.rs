use puffersecuresigner::io::{key_management, runtime};
use std::{path::Path, process::Command};

#[test]
fn native_and_container_paths_preserve_storage_and_rpc_boundaries() {
    for emulated in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", "runtime_child", "--nocapture"])
            .current_dir(temp.path())
            .env("ATAKIT_TEST_RUNTIME_CHILD", "1")
            .env("ATAKIT_PORTAL_SOCKET_PATH", "/obsolete-portal.sock")
            .env("SESSION_REGISTRY_RPC_URL", "https://chain.example")
            .env("CVM_AGENT_SOCKET_PATH", "/obsolete-agent.sock")
            .env_remove("EMULATOR_ROOTFS")
            .env_remove("EMULATOR_RPC_URL");
        if emulated {
            child
                .env("EMULATOR_ROOTFS", temp.path().join("root"))
                .env("EMULATOR_RPC_URL", "http://127.0.0.1:8546");
        }
        let output = child.output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let data = if emulated {
            temp.path().join("root/data")
        } else {
            temp.path().join("data")
        };
        assert!(data.join("keys/eth_keys/1234").is_file());
        assert!(data.join("keys/bls_keys/5678").is_file());
        if emulated {
            assert!(!temp.path().join("data").exists());
        }
    }
}

#[test]
fn runtime_child() {
    if std::env::var_os("ATAKIT_TEST_RUNTIME_CHILD").is_none() {
        return;
    }
    let root = std::env::var_os("EMULATOR_ROOTFS");
    let expected = root
        .as_ref()
        .map(|root| Path::new(root).join("run/atakit-portal.sock"))
        .unwrap_or_else(|| "/run/atakit-portal.sock".into());
    assert_eq!(
        atakit_portal_client::PortalClient::default().socket(),
        expected
    );
    assert_eq!(
        runtime::session_registry_rpc_url().unwrap(),
        if root.is_some() {
            "http://127.0.0.1:8546"
        } else {
            "https://chain.example"
        }
    );
    key_management::write_eth_key(&"1234".into(), &"abcd".into()).unwrap();
    assert_eq!(
        key_management::read_eth_key("1234").unwrap(),
        vec![0xab, 0xcd]
    );
    assert!(key_management::eth_key_exists("1234"));
    assert!(
        key_management::list_eth_keys()
            .unwrap()
            .contains(&"1234".into())
    );
    key_management::write_bls_keystore(&"5678".into(), &[1u8; 32], &"test-password".into())
        .unwrap();
    assert_eq!(
        key_management::read_bls_keystore(&"5678".into(), &"test-password".into()).unwrap(),
        vec![1u8; 32]
    );
    let slash = runtime::storage_path("./data/slashing/");
    assert!(slash.ends_with("data/slashing"));
    if let Some(root) = root {
        assert!(slash.starts_with(Path::new(&root)));
    }
}
