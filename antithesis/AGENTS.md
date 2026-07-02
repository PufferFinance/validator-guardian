This directory contains files relevant to running tests in Antithesis.

Use the `antithesis-setup` skill to scaffold and manage this directory. Use the `antithesis-research` skill to analyze the system and build a property catalog. Use the `antithesis-workload` skill to implement assertions and test commands. Use the `antithesis-launch` skill to build, validate, and submit Antithesis runs — do not run `snouty launch` directly.

**Launching runs**
Use the `antithesis-launch` skill to build, validate, and submit runs — do not run `snouty launch` directly. (For reference, launch ultimately runs `snouty launch --json --webhook basic_test --config antithesis/config` after a `compose build`.)

**snouty validate**
Use this command to quickly validate changes to the Antithesis scaffolding. See `snouty validate --help` for details.

**setup-complete.sh**
Inject this script into a container to notify Antithesis that setup is complete. It writes the `setup_complete` event to `$ANTITHESIS_OUTPUT_DIR/sdk.jsonl`. It should only run once the system under test is ready for testing. Antithesis (and `snouty validate`) will not run any test commands until it receives this event. In this harness the `workload` container runs it after the guardian and session-registry are healthy.

**config**
This directory contains the `docker-compose.yaml` file used to bring up this system within the Antithesis environment, along with any closely related config files. Snouty will push tagged images, consume this config directory, and launch the run.

**scratchbook**
This directory is the Antithesis scratchbook for the codebase. It contains documents such as system analysis, property catalogs, topology plans, per-property evidence files (in `scratchbook/properties/`), property relationship maps, and other persistent integration notes. Keep it up to date as Antithesis-related decisions change.

**test**
This directory contains test templates. A test template is a directory containing test command executable files. Each test command must have a valid prefix: `parallel_driver_, singleton_driver_, serial_driver_, first_, eventually_, finally_, anytime_`. Prefixes constrain when and how commands are composed in a single timeline. Files or subdirectories prefixed with `helper_` are ignored by Antithesis and can be used for helper scripts kept alongside the commands. It is baked into the `workload` image at `/opt/antithesis/test/v1/`.

---

## Harness layout (this project)

SUT: the **`guardian`** binary of the `puffersecuresigner` crate (see `scratchbook/sut-analysis.md`). The harness has three services (see `scratchbook/deployment-topology.md`):

| Service | Role | Image (Dockerfile stage) | Notes |
|---|---|---|---|
| `guardian` | SUT | `antithesis/Dockerfile` target `guardian` | Instrumented Rust build (Antithesis coverage + `antithesis_sdk`), exposes `/symbols`, serves `:9001`, exec-form entrypoint. |
| `session-registry` | dependency | `antithesis/Dockerfile` target `session-registry` | `anvil` (foundry image) on `:8545`, the on-chain verifier the guardian calls via `eth_call`. |
| `workload` | client / driver | `antithesis/Dockerfile` target `workload` | Waits for the other two to be healthy, emits `setup_complete`, then idles. Test templates land in `/opt/antithesis/test/v1/`. |

### Instrumentation

- `antithesis_sdk` is a hard dependency of the `puffersecuresigner` crate. It is a no-op / low overhead outside Antithesis, so it is safe in production builds.
- Coverage instrumentation (`antithesis-instrumentation` + sancov `rustflags`) is gated behind the `antithesis_instr` Cargo feature and only enabled for the `guardian` image build. See the header comments in `antithesis/Dockerfile`.
- Bootstrap property: `assert_reachable!("guardian startup path executed", …)` in `src/bin/guardian.rs` — a guaranteed-to-run startup path proving the SDK, cataloging, and instrumentation are wired.
- Symbols for the guardian are exposed by symlinking the (unstripped, `--build-id`, DWARF) binary into `/symbols/`.

### Deferred to `antithesis-workload` (intentionally NOT built here)

Setup delivers a **ready, idle** harness. The following are workload-scoped and left as clearly-marked hooks:

- **Controllable mock CVM agent** (unix socket `/app/cvm-agent.sock`). The guardian never calls it at idle, so setup does not ship one. The workload skill must implement the `automata-cvm-agent` `sign_message` protocol with a fixed/registerable session key (see `scratchbook/deployment-topology.md`, "mock CVM agent" and evaluation refinement R7). Until then, do not exercise `POST /eth/v1/keygen` / verify-session custody paths.
- **Contract deployment on anvil.** `session-registry` comes up as a healthy, empty anvil node. Deploying `SessionRegistry` + `GuardianModule` from `puffer-contracts` and rotating in the guardian's runtime enclave address is workload-driven (topology R2/R7). The deploy hook is marked in `antithesis/session-registry/entrypoint.sh`.
- **Real test-template driver commands.** `antithesis/test/` currently holds only a placeholder. The workload skill turns the `workload` container into the `puffersecuresigner`-backed Rust driver and adds `*_driver_` / `anytime_` commands.
