use atakit_portal_client::PortalClient;
use std::{path::Path, process::Command};

#[test]
fn default_socket_uses_emulator_root_only_when_nonempty() {
    for root in [None, Some(""), Some("/tmp/native root/")] {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "socket_child", "--nocapture"])
            .env("ATAKIT_SOCKET_TEST_CHILD", "1")
            .env_remove("EMULATOR_ROOTFS")
            .env("PORTAL_SOCKET", "/ignored.sock");
        let expected = match root {
            Some(root) if !root.is_empty() => Path::new(root).join("run/atakit-portal.sock"),
            _ => "/run/atakit-portal.sock".into(),
        };
        command.env("ATAKIT_SOCKET_TEST_EXPECTED", expected);
        if let Some(root) = root {
            command.env("EMULATOR_ROOTFS", root);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn socket_child() {
    if std::env::var_os("ATAKIT_SOCKET_TEST_CHILD").is_none() {
        return;
    }
    let expected = std::env::var_os("ATAKIT_SOCKET_TEST_EXPECTED").unwrap();
    assert_eq!(PortalClient::default().socket(), Path::new(&expected));
    assert_eq!(
        PortalClient::new("/explicit.sock").socket(),
        Path::new("/explicit.sock")
    );
}
