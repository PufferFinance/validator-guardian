//! Antithesis test command for property `no-request-authentication`.
//!
//! The guardian router has no authentication middleware (see
//! `antithesis/scratchbook/properties/no-request-authentication.md`): the
//! security model rests entirely on network isolation. This command IS an
//! unauthenticated caller. It probes a privileged guardian endpoint with no
//! credentials and, when the endpoint serves the request (any status other than
//! 401/403), fires a `Reachable` documenting that a privileged endpoint served an
//! unauthenticated request.
//!
//! Regression tripwire: if an authentication layer is ever added, an
//! unauthenticated request would be rejected with 401/403 before reaching
//! endpoint logic, the `Reachable` would stop firing, and Antithesis would flag
//! the cataloged assertion as unreachable.

use antithesis_sdk::{assert_reachable, random};
use serde_json::json;

/// Privileged guardian endpoints.
///
/// Menu axis (interesting-values): an authentication-presence property has no
/// bounded numeric inputs, so the "menu" is this fixed vocabulary of privileged
/// endpoints rather than a value range. Drawing it via the Antithesis RNG means
/// different timelines probe different endpoints, so across many timelines every
/// privileged endpoint is shown to serve unauthenticated callers.
#[derive(Clone, Copy)]
enum Probe {
    KeygenPost,      // POST /eth/v1/keygen            (well-formed body -> reaches handler logic)
    KeygenList,      // GET  /eth/v1/keygen
    ValidateCustody, // POST /guardian/v1/validate-custody
    SignExit,        // POST /guardian/v1/sign-exit
}

impl Probe {
    fn name(self) -> &'static str {
        match self {
            Probe::KeygenPost => "keygen",
            Probe::KeygenList => "list-eth-keys",
            Probe::ValidateCustody => "validate-custody",
            Probe::SignExit => "sign-exit",
        }
    }
    fn method(self) -> &'static str {
        match self {
            Probe::KeygenList => "GET",
            _ => "POST",
        }
    }
    fn path(self) -> &'static str {
        match self {
            Probe::KeygenPost | Probe::KeygenList => "/eth/v1/keygen",
            Probe::ValidateCustody => "/guardian/v1/validate-custody",
            Probe::SignExit => "/guardian/v1/sign-exit",
        }
    }
}

#[tokio::main]
async fn main() {
    // Registers this binary's assertions in the Antithesis catalog. No-op outside
    // Antithesis.
    antithesis_sdk::antithesis_init();

    let base = std::env::var("GUARDIAN_BASE_URL")
        .unwrap_or_else(|_| "http://guardian:9001".to_string());
    let client = reqwest::Client::new();

    let probes = [
        Probe::KeygenPost,
        Probe::KeygenList,
        Probe::ValidateCustody,
        Probe::SignExit,
    ];
    let probe = *random::random_choice(&probes).expect("probe menu is non-empty");
    let url = format!("{}{}", base, probe.path());

    // Fault tolerance: retry a bounded number of times on transient/connection
    // errors, which are expected under Antithesis fault injection (partitions,
    // node kills). A connection error is NOT a bug for this property, so we never
    // exit non-zero on it — Antithesis re-runs the command to make more progress.
    let mut last_err: Option<reqwest::Error> = None;
    for attempt in 0..5u32 {
        let req = match probe {
            Probe::KeygenPost => client.post(&url).json(&json!({
                "block_number": 0,
                "guardian_module_address": "0x0000000000000000000000000000000000000000",
                "chain_id": 31337
            })),
            Probe::KeygenList => client.get(&url),
            // Minimal bodies: an auth layer, if present, rejects with 401/403 at the
            // routing layer BEFORE body validation, so a 422 here still proves no
            // auth gate intervened. Valid custody/exit bodies require the deferred
            // CVM-agent mock and are unnecessary to observe authentication presence.
            Probe::ValidateCustody | Probe::SignExit => client.post(&url).json(&json!({})),
        };

        match req.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                // "Served" means no authentication gate rejected the request, i.e.
                // status is not 401/403. A non-2xx business error still counts:
                // e.g. keygen returns 500 here because the CVM agent is not mocked
                // yet (deferred to a later property), and validate-custody/sign-exit
                // return 422 from body validation — in all cases the privileged
                // endpoint processed the unauthenticated request rather than
                // rejecting it for missing credentials.
                if status != 401 && status != 403 {
                    assert_reachable!(
                        "guardian served a privileged request with no authentication present",
                        &json!({
                            "endpoint": probe.name(),
                            "method": probe.method(),
                            "path": probe.path(),
                            "status": status
                        })
                    );
                    println!(
                        "probe {} {} -> {} (served without authentication)",
                        probe.method(),
                        probe.path(),
                        status
                    );
                } else {
                    println!(
                        "probe {} {} -> {} (auth rejection)",
                        probe.method(),
                        probe.path(),
                        status
                    );
                }
                return;
            }
            Err(e) => {
                eprintln!(
                    "attempt {} transient error (expected under fault injection): {}",
                    attempt, e
                );
                last_err = Some(e);
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }

    // All attempts hit transient errors (e.g. partition / node kill). Not a bug
    // for this property; exit 0 and let Antithesis re-run the command later.
    if let Some(e) = last_err {
        eprintln!("all probe attempts failed transiently: {} — exiting 0 (not a bug)", e);
    }
}
