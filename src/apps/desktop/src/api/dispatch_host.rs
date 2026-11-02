//! Target-side detached-dispatch adapter for account device RPC.
//!
//! Desktop deliberately delegates execution to the same `openbitfun dispatch`
//! runner used by SSH and CLI Peer Host. This keeps one durable job/session
//! owner and avoids creating a second desktop-only dispatch implementation.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context};
use serde_json::Value;
use tokio::io::AsyncWriteExt;

const TARGET_COMMAND_TIMEOUT: Duration = Duration::from_secs(110);
const MAX_TARGET_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// Runs a target command for `source_device_id`, the relay-authenticated
/// sender. The sender is bound here so the target never trusts a self-reported
/// device.
pub(crate) async fn dispatch(
    command: &str,
    args: Value,
    source_device_id: &str,
) -> anyhow::Result<Value> {
    let verb = target_cli_verb(command)
        .ok_or_else(|| anyhow!("Unknown detached dispatch target command '{command}'"))?;
    let args = openbitfun_services_core::dispatch_contract::bind_dispatch_source_device(
        command,
        args,
        source_device_id,
    );
    let executable = discover_cli().ok_or_else(|| {
        anyhow!(
            "OpenBitFun CLI dispatch runner is not installed on this device; install `openbitfun` in ~/.local/bin or PATH"
        )
    })?;
    invoke_cli(&executable, verb, args).await
}

/// Runs a target command that arrived over the cross-account connection
/// `connection_id`, bound to that connection. Commands a connection may not
/// send are refused before the runner starts.
pub(crate) async fn dispatch_for_connection(
    command: &str,
    args: Value,
    connection_id: &str,
) -> anyhow::Result<Value> {
    let verb = target_cli_verb(command)
        .ok_or_else(|| anyhow!("Unknown detached dispatch target command '{command}'"))?;
    let args = openbitfun_services_core::dispatch_contract::bind_dispatch_source_connection(
        command,
        args,
        connection_id,
    )
    .map_err(anyhow::Error::msg)?;
    let executable = discover_cli().ok_or_else(|| {
        anyhow!(
            "OpenBitFun CLI dispatch runner is not installed on this device; install `openbitfun` in ~/.local/bin or PATH"
        )
    })?;
    invoke_cli(&executable, verb, args).await
}

/// Runs `openbitfun dispatch <verb>` as this device's owner: no source is
/// bound, so the runner answers for every job on this device.
pub(crate) async fn run_local(verb: &'static str, args: Value) -> anyhow::Result<Value> {
    let executable = discover_cli().ok_or_else(|| {
        anyhow!(
            "OpenBitFun CLI dispatch runner is not installed on this device; install `openbitfun` in ~/.local/bin or PATH"
        )
    })?;
    invoke_cli(&executable, verb, args).await
}

/// Answers dispatch calls from paired devices of other accounts. What each
/// connection may reach is decided by the runner from this device's grants.
pub(crate) struct ConnectionDispatchHandler;

#[async_trait::async_trait]
impl openbitfun_core::service::remote_connect::connection_rpc::ConnectionRequestHandler
    for ConnectionDispatchHandler
{
    async fn handle(
        &self,
        caller: &openbitfun_core::service::remote_connect::connection_rpc::ConnectionCaller,
        command: &str,
        args: Value,
    ) -> Result<Value, String> {
        dispatch_for_connection(command, args, &caller.connection_id)
            .await
            .map_err(|error| format!("{error:#}"))
    }
}

pub(crate) fn is_target_command(command: &str) -> bool {
    target_cli_verb(command).is_some()
}

fn target_cli_verb(command: &str) -> Option<&'static str> {
    match command {
        "dispatch_target_probe" => Some("probe"),
        "dispatch_target_submit" => Some("submit"),
        "dispatch_target_status" => Some("status"),
        "dispatch_target_cancel" => Some("cancel"),
        "dispatch_target_catalog" => Some("catalog"),
        "dispatch_target_list" => Some("list"),
        "dispatch_target_answer" => Some("answer"),
        "dispatch_target_append" => Some("append"),
        "dispatch_target_continue" => Some("continue"),
        "dispatch_target_query" => Some("query"),
        "dispatch_target_workspace_provision" => Some("__workspace_provision"),
        "dispatch_target_workspace_bundle_begin" => Some("__workspace_bundle_begin"),
        "dispatch_target_workspace_bundle_chunk" => Some("__workspace_bundle_chunk"),
        "dispatch_target_workspace_bundle_commit" => Some("__workspace_bundle_commit"),
        "dispatch_target_workspace_sync" => Some("__workspace_sync"),
        "dispatch_target_workspace_sync_chunk" => Some("__workspace_sync_chunk"),
        _ => None,
    }
}

async fn invoke_cli(executable: &Path, verb: &str, args: Value) -> anyhow::Result<Value> {
    let request = serde_json::to_vec(&args).context("serialize target dispatch request")?;
    let mut child = openbitfun_core::util::process_manager::create_tokio_command(executable)
        .arg("dispatch")
        .arg(verb)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("start dispatch runner {}", executable.display()))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("Dispatch runner stdin was unavailable"))?;
    stdin
        .write_all(&request)
        .await
        .context("write target dispatch request")?;
    stdin.shutdown().await?;
    drop(stdin);

    let output = tokio::time::timeout(TARGET_COMMAND_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| anyhow!("Target dispatch command timed out"))?
        .context("wait for target dispatch command")?;
    if output.stdout.len() > MAX_TARGET_RESPONSE_BYTES
        || output.stderr.len() > MAX_TARGET_RESPONSE_BYTES
    {
        anyhow::bail!("Target dispatch response exceeded the 4 MiB safety limit");
    }
    let stdout = std::str::from_utf8(&output.stdout)
        .context("Target dispatch runner returned non-UTF-8 output")?
        .trim();
    let response: Value = serde_json::from_str(stdout).with_context(|| {
        format!(
            "Target dispatch runner returned invalid JSON: {}",
            bounded_text(stdout)
        )
    })?;
    if !output.status.success() {
        let message = response
            .get("error")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| String::from_utf8_lossy(&output.stderr).trim().to_string());
        anyhow::bail!(
            "{}",
            if message.is_empty() {
                "Target dispatch runner failed".to_string()
            } else {
                message
            }
        );
    }
    Ok(response)
}

fn discover_cli() -> Option<PathBuf> {
    let executable_dir = std::env::current_exe()
        .ok()
        .and_then(|current| current.parent().map(Path::to_path_buf));
    cli_candidates(
        executable_dir.as_deref(),
        dirs::home_dir().as_deref(),
        std::env::var_os("PATH").as_deref(),
    )
    .into_iter()
    .find_map(resolve_cli_candidate)
}

/// Where the executor is looked for, in order. The copy shipped next to the
/// desktop executable comes first, so an installed app answers with its own
/// version; a separately installed CLI is the fallback.
fn cli_candidates(
    executable_dir: Option<&Path>,
    home: Option<&Path>,
    path: Option<&std::ffi::OsStr>,
) -> Vec<PathBuf> {
    let executable_name = if cfg!(windows) {
        "openbitfun.exe"
    } else {
        "openbitfun"
    };
    let mut candidates = Vec::new();
    if let Some(directory) = executable_dir {
        candidates.push(directory.join(executable_name));
    }
    if let Some(home) = home {
        candidates.push(home.join(".local").join("bin").join(executable_name));
    }
    if let Some(path) = path {
        candidates
            .extend(std::env::split_paths(path).map(|directory| directory.join(executable_name)));
    }
    candidates
}

fn resolve_cli_candidate(candidate: PathBuf) -> Option<PathBuf> {
    let canonical = candidate.canonicalize().ok()?;
    std::fs::symlink_metadata(&canonical)
        .is_ok_and(|metadata| !metadata.file_type().is_symlink() && metadata.is_file())
        .then_some(canonical)
}

fn bounded_text(value: &str) -> String {
    const MAX_CHARS: usize = 240;
    let mut result = value.chars().take(MAX_CHARS).collect::<String>();
    if value.chars().count() > MAX_CHARS {
        result.push_str("...");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbound_controller_commands_are_not_target_commands() {
        assert!(is_target_command("dispatch_target_status"));
        assert!(!is_target_command("dispatch_status"));
        assert!(!is_target_command("account_device_rpc"));
    }

    /// The registry marks every `dispatch_target_*` name as host control plane
    /// (routed to the dispatch runner before the peer bridge); this table must
    /// answer exactly that family, no more and no less.
    #[test]
    fn target_command_family_matches_registry() {
        use openbitfun_product_domains::remote_surface::{operations, PeerStance};
        for op in operations() {
            let registry_says_target = op.id.starts_with("dispatch_target_");
            assert_eq!(
                is_target_command(op.id),
                registry_says_target,
                "{} target-command classification disagrees with the registry",
                op.id
            );
            if registry_says_target {
                assert_eq!(op.peer, PeerStance::HostControlPlane, "{}", op.id);
            }
        }
    }

    #[test]
    fn the_bundled_executor_is_preferred_over_installed_ones() {
        let name = if cfg!(windows) {
            "openbitfun.exe"
        } else {
            "openbitfun"
        };
        let app = Path::new("/Applications/OpenBitFun.app/Contents/MacOS");
        let home = Path::new("/Users/someone");
        let path = std::env::join_paths(["/opt/bin", "/usr/local/bin"]).expect("PATH");

        assert_eq!(
            cli_candidates(Some(app), Some(home), Some(path.as_os_str())),
            vec![
                app.join(name),
                home.join(".local").join("bin").join(name),
                Path::new("/opt/bin").join(name),
                Path::new("/usr/local/bin").join(name),
            ]
        );
        assert_eq!(
            cli_candidates(None, Some(home), None),
            vec![home.join(".local").join("bin").join(name)]
        );
    }

    #[cfg(unix)]
    #[test]
    fn cli_discovery_accepts_a_symlink_to_a_regular_binary() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let binary = temp.path().join("openbitfun-real");
        std::fs::write(&binary, b"binary").expect("binary");
        let link = temp.path().join("openbitfun");
        symlink(&binary, &link).expect("symlink");

        assert_eq!(
            resolve_cli_candidate(link),
            Some(binary.canonicalize().expect("canonical binary"))
        );
    }
}
