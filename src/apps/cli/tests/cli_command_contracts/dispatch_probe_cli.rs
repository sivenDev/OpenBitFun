//! `openbitfun dispatch probe` as a delegating device reads it: an installed
//! CLI advertises that it takes delegated assistant tasks and can run
//! external agents for them.

#[path = "../support/mod.rs"]
mod support;

use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use support::CliTestEnvironment;

#[test]
fn targets_advertise_delegation_and_external_agents_for_assistant_tasks() {
    let environment = CliTestEnvironment::new();
    let mut command = environment.std_command();
    command.args(["dispatch", "probe"]).stdin(Stdio::null());
    let output = support::command_output_with_timeout(&mut command, Duration::from_secs(60));
    assert!(
        output.status.success(),
        "dispatch probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let probe: Value = serde_json::from_slice(&output.stdout).expect("probe JSON");
    let capabilities: Vec<&str> = probe["capabilities"]
        .as_array()
        .expect("capabilities")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        capabilities.contains(&"agent_delegation_v1"),
        "{capabilities:?}"
    );
    assert!(
        capabilities.contains(&"external_agent_executor_v1"),
        "{capabilities:?}"
    );
}
