//! OpenBitFun-managed installs of the npm ACP adapters that built-in clients launch.
//!
//! The Claude Code and Codex presets are persisted as `npx --yes <adapter>@latest`.
//! Run as-is, npx asks the registry about `@latest` on every launch and installs a
//! new release inside the launch, where the ACP handshake timeout kills it. The
//! store installs the launch spec into a directory OpenBitFun owns, starts the
//! installed adapter directly, and looks for new releases after the client is up;
//! the next launch picks them up. The persisted command does not change: older
//! builds keep running it through npx, and a launch whose install fails for any
//! reason other than a timeout falls back to it.
//!
//! Layout of one store entry, keyed by the spec text:
//!
//! ```text
//! <root>/<spec key>/current         name of the install launches use
//! <root>/<spec key>/install-<id>/   an `npm install --prefix` tree
//! ```
//!
//! Every install gets a fresh directory and `current` is replaced atomically, so
//! an adapter that is already running never sees its files change. The previous
//! install is kept for the sessions still running it; older ones are pruned.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use dashmap::DashMap;
use log::{debug, info, warn};
use openbitfun_core::service::remote_ssh::{SSHCommandOptions, SSHConnectionManager};
use openbitfun_core::util::errors::OpenBitFunError;
use tokio::sync::Mutex;

use super::config::AcpClientConfig;
use super::remote_shell::{remote_user_shell_command, render_remote_env_assignments, shell_escape};
use super::requirements::{
    apply_command_environment, command_error_summary, remote_command_error_summary,
    resolve_configured_command, AcpAdapterSpec, ADAPTER_DOWNLOAD_TIMEOUT,
};

const CURRENT_FILE: &str = "current";
const INSTALL_DIR_PREFIX: &str = "install-";
const INSTALL_FLAGS: &[&str] = &["--no-audit", "--no-fund", "--omit=dev"];
/// How often launches of an adapter that follows a tag ask for a newer release.
const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const UPDATE_LOOKUP_TIMEOUT: Duration = Duration::from_secs(60);
/// An install directory without a usable adapter this old is a crashed install.
const ABANDONED_INSTALL_AGE: Duration = Duration::from_secs(24 * 60 * 60);
/// The store root on an SSH host, as its shell expands it.
const REMOTE_STORE_ROOT: &str = "$HOME/.openbitfun/acp-adapters";

/// A configured npx launch of a built-in adapter, split into what npm installs
/// and what the adapter itself receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AdapterLaunch {
    package: String,
    spec: String,
    bin: String,
    /// The npm that ships beside the configured npx.
    npm: String,
    /// npx options ahead of the spec, such as `--registry=…`, minus `--yes`.
    npm_options: Vec<String>,
    /// Everything after the spec: the adapter's own arguments.
    adapter_args: Vec<String>,
}

/// Finds the adapter spec in a configured npx launch.
pub(crate) fn npx_adapter_launch(
    config: &AcpClientConfig,
    adapter: &AcpAdapterSpec<'_>,
) -> Option<AdapterLaunch> {
    let npm = npm_beside_npx(config.command.trim())?;
    let index = config
        .args
        .iter()
        .position(|arg| is_package_spec(arg, adapter.package))?;
    let options = &config.args[..index];
    // With `--package`, npx runs a command out of the named packages and the
    // positional spec means something else entirely.
    if options
        .iter()
        .any(|arg| arg == "-p" || arg.starts_with("--package"))
    {
        return None;
    }
    Some(AdapterLaunch {
        package: adapter.package.to_string(),
        spec: config.args[index].clone(),
        bin: adapter.bin.to_string(),
        npm,
        npm_options: options
            .iter()
            .filter(|arg| !matches!(arg.as_str(), "--yes" | "-y"))
            .cloned()
            .collect(),
        adapter_args: config.args[index + 1..].to_vec(),
    })
}

/// The npm installed beside `command`, or `None` when the command is not npx.
/// String-based because a remote command is a POSIX path on every client OS.
fn npm_beside_npx(command: &str) -> Option<String> {
    let name_start = command.rfind(['/', '\\']).map_or(0, |index| index + 1);
    let (directory, name) = command.split_at(name_start);
    let extension = name.get(3..)?;
    let is_npx = name.get(..3)?.eq_ignore_ascii_case("npx")
        && (extension.is_empty() || extension.starts_with('.'));
    is_npx.then(|| format!("{directory}npm{extension}"))
}

fn is_package_spec(arg: &str, package: &str) -> bool {
    arg.strip_prefix(package)
        .is_some_and(|version| version.is_empty() || version.starts_with('@'))
}

impl AdapterLaunch {
    pub(crate) fn spec(&self) -> &str {
        &self.spec
    }

    /// Whether a later release can satisfy the spec: a tag, a range, or no
    /// version at all, rather than one exact version.
    fn follows_releases(&self) -> bool {
        let version = self.spec[self.package.len()..].trim_start_matches('@');
        semver::Version::parse(version).is_err()
    }

    /// One store entry per spec text, as a directory name.
    fn store_key(&self) -> String {
        self.spec
            .trim_start_matches('@')
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | '@') {
                    ch
                } else {
                    '+'
                }
            })
            .collect()
    }

    fn install_args(&self, prefix: &str) -> Vec<String> {
        let mut args = self.npm_options.clone();
        args.extend(["install", "--prefix", prefix].map(str::to_string));
        args.extend(INSTALL_FLAGS.iter().map(|flag| (*flag).to_string()));
        args.push(self.spec.clone());
        args
    }

    fn view_args(&self) -> Vec<String> {
        let mut args = self.npm_options.clone();
        args.extend(["view".to_string(), self.spec.clone(), "version".to_string()]);
        args
    }

    /// The configured launch with npx replaced by the installed adapter.
    pub(crate) fn installed_launch(&self, config: &AcpClientConfig, bin: &str) -> AcpClientConfig {
        AcpClientConfig {
            command: bin.to_string(),
            args: self.adapter_args.clone(),
            ..config.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstalledAdapter {
    /// The adapter executable, on the host that runs it.
    pub(crate) bin: String,
    pub(crate) version: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AdapterInstallError {
    /// The install outlasted [`ADAPTER_DOWNLOAD_TIMEOUT`].
    TimedOut,
    /// npm ran and failed, or could not be run.
    Failed(String),
}

impl AdapterInstallError {
    pub(crate) fn into_error(self, client_id: &str, spec: &str) -> OpenBitFunError {
        match self {
            Self::TimedOut => OpenBitFunError::service(format!(
                "Installing the ACP adapter '{}' for client '{}' did not finish within {}s. Check the network or npm registry and try again.",
                spec,
                client_id,
                ADAPTER_DOWNLOAD_TIMEOUT.as_secs()
            )),
            Self::Failed(detail) => OpenBitFunError::service(format!(
                "Failed to install the ACP adapter '{}' for client '{}': {}",
                spec, client_id, detail
            )),
        }
    }
}

pub(crate) struct AdapterStore {
    root: PathBuf,
    /// One install at a time per entry, so concurrent launches of a client wait
    /// for a single download instead of starting their own.
    entry_locks: DashMap<String, Arc<Mutex<()>>>,
    /// When each local or remote entry last asked the registry for a release.
    update_checks: DashMap<String, Instant>,
}

impl AdapterStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            entry_locks: DashMap::new(),
            update_checks: DashMap::new(),
        }
    }

    fn entry_lock(&self, key: &str) -> Arc<Mutex<()>> {
        self.entry_locks.entry(key.to_string()).or_default().clone()
    }

    /// Records a registry check for `key` and reports whether one was due.
    fn claim_update_check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut due = true;
        self.update_checks
            .entry(key.to_string())
            .and_modify(|checked_at| {
                due = now.duration_since(*checked_at) >= UPDATE_CHECK_INTERVAL;
                if due {
                    *checked_at = now;
                }
            })
            .or_insert(now);
        due
    }

    /// The install launches of this spec use, if the store has one.
    pub(crate) fn installed(&self, launch: &AdapterLaunch) -> Option<InstalledAdapter> {
        let entry = self.root.join(launch.store_key());
        let current = std::fs::read_to_string(entry.join(CURRENT_FILE)).ok()?;
        installed_in(&entry.join(current.trim()), launch)
    }

    /// The installed adapter, installing the spec first when the store has none.
    pub(crate) async fn ensure_installed(
        &self,
        launch: &AdapterLaunch,
        env: &HashMap<String, String>,
    ) -> Result<InstalledAdapter, AdapterInstallError> {
        let lock = self.entry_lock(&launch.store_key());
        let _guard = lock.lock().await;
        if let Some(installed) = self.installed(launch) {
            return Ok(installed);
        }
        self.install(launch, env).await
    }

    /// Installs the release the spec resolves to now, unless it is already the
    /// current install.
    pub(crate) async fn update(
        &self,
        launch: &AdapterLaunch,
        env: &HashMap<String, String>,
    ) -> Result<InstalledAdapter, AdapterInstallError> {
        let lock = self.entry_lock(&launch.store_key());
        let _guard = lock.lock().await;
        if let Some(current) = self.installed(launch) {
            if !launch.follows_releases() {
                return Ok(current);
            }
            let output = run_npm(launch, env, launch.view_args(), UPDATE_LOOKUP_TIMEOUT).await?;
            let latest = parse_view_version(&String::from_utf8_lossy(&output));
            if latest.is_none() || latest == current.version {
                return Ok(current);
            }
        }
        self.install(launch, env).await
    }

    /// Looks for a newer release after a launch, at most once per
    /// [`UPDATE_CHECK_INTERVAL`] per spec. The next launch uses what it finds.
    pub(crate) fn update_in_background(
        self: &Arc<Self>,
        client_id: &str,
        launch: AdapterLaunch,
        env: HashMap<String, String>,
    ) {
        if !launch.follows_releases() || !self.claim_update_check(&launch.store_key()) {
            return;
        }
        let store = Arc::clone(self);
        let client_id = client_id.to_string();
        tokio::spawn(async move {
            match store.update(&launch, &env).await {
                Ok(installed) => debug!(
                    "ACP adapter release check finished: id={} spec={} version={:?}",
                    client_id, launch.spec, installed.version
                ),
                Err(error) => warn!(
                    "ACP adapter release check failed: id={} spec={} error={:?}",
                    client_id, launch.spec, error
                ),
            }
        });
    }

    async fn install(
        &self,
        launch: &AdapterLaunch,
        env: &HashMap<String, String>,
    ) -> Result<InstalledAdapter, AdapterInstallError> {
        let entry = self.root.join(launch.store_key());
        let name = format!("{INSTALL_DIR_PREFIX}{}", uuid::Uuid::new_v4().simple());
        let directory = entry.join(&name);
        let started_at = Instant::now();
        let result = install_into(&directory, launch, env).await;
        let installed = match result {
            Ok(installed) => installed,
            Err(error) => {
                let _ = tokio::fs::remove_dir_all(&directory).await;
                return Err(error);
            }
        };
        // Freshly installed is as current as a registry check would make it.
        self.update_checks
            .insert(launch.store_key(), Instant::now());

        let previous = std::fs::read_to_string(entry.join(CURRENT_FILE))
            .ok()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty());
        if let Some(previous_install) = previous
            .as_deref()
            .and_then(|previous| installed_in(&entry.join(previous), launch))
            .filter(|current| current.version == installed.version)
        {
            let _ = tokio::fs::remove_dir_all(&directory).await;
            return Ok(previous_install);
        }

        write_current(&entry, &name)
            .await
            .map_err(|error| AdapterInstallError::Failed(error.to_string()))?;
        info!(
            "ACP adapter installed: spec={} version={:?} duration_ms={}",
            launch.spec,
            installed.version,
            started_at.elapsed().as_millis()
        );
        prune_installs(&entry, launch, &name, previous.as_deref()).await;
        Ok(installed)
    }
}

async fn install_into(
    directory: &Path,
    launch: &AdapterLaunch,
    env: &HashMap<String, String>,
) -> Result<InstalledAdapter, AdapterInstallError> {
    let failed = |error: std::io::Error| AdapterInstallError::Failed(error.to_string());
    tokio::fs::create_dir_all(directory).await.map_err(failed)?;
    // Without a manifest of its own, npm would install into an enclosing project.
    tokio::fs::write(directory.join("package.json"), "{\"private\":true}\n")
        .await
        .map_err(failed)?;
    let prefix = directory.to_string_lossy();
    run_npm(
        launch,
        env,
        launch.install_args(&prefix),
        ADAPTER_DOWNLOAD_TIMEOUT,
    )
    .await?;
    installed_in(directory, launch).ok_or_else(|| {
        AdapterInstallError::Failed(format!(
            "npm finished without installing the '{}' executable",
            launch.bin
        ))
    })
}

fn installed_in(directory: &Path, launch: &AdapterLaunch) -> Option<InstalledAdapter> {
    let bin_directory = directory.join("node_modules").join(".bin");
    let bin = if cfg!(windows) {
        bin_directory.join(format!("{}.cmd", launch.bin))
    } else {
        bin_directory.join(&launch.bin)
    };
    if !bin.is_file() {
        return None;
    }
    let manifest = directory
        .join("node_modules")
        .join(&launch.package)
        .join("package.json");
    let version = std::fs::read(manifest)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|manifest| manifest.get("version")?.as_str().map(str::to_string));
    Some(InstalledAdapter {
        bin: bin.to_string_lossy().into_owned(),
        version,
    })
}

async fn write_current(entry: &Path, name: &str) -> std::io::Result<()> {
    let staged = entry.join(format!("{CURRENT_FILE}.{}", uuid::Uuid::new_v4().simple()));
    tokio::fs::write(&staged, format!("{name}\n")).await?;
    let result = tokio::fs::rename(&staged, entry.join(CURRENT_FILE)).await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&staged).await;
    }
    result
}

/// Keeps the current install and the one before it, which sessions started
/// earlier may still be running. Directories without a usable adapter are left
/// alone unless they are old enough to be a crashed install rather than one in
/// progress in another OpenBitFun process.
async fn prune_installs(
    entry: &Path,
    launch: &AdapterLaunch,
    current: &str,
    previous: Option<&str>,
) {
    let Ok(mut entries) = tokio::fs::read_dir(entry).await else {
        return;
    };
    while let Ok(Some(item)) = entries.next_entry().await {
        let name = item.file_name().to_string_lossy().into_owned();
        if !name.starts_with(INSTALL_DIR_PREFIX)
            || name == current
            || Some(name.as_str()) == previous
        {
            continue;
        }
        let path = item.path();
        let abandoned = item
            .metadata()
            .await
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age >= ABANDONED_INSTALL_AGE);
        if installed_in(&path, launch).is_some() || abandoned {
            if let Err(error) = tokio::fs::remove_dir_all(&path).await {
                // An adapter that is still running can hold its files open.
                debug!(
                    "Skipped pruning ACP adapter install: path={} error={}",
                    path.display(),
                    error
                );
            }
        }
    }
}

async fn run_npm(
    launch: &AdapterLaunch,
    env: &HashMap<String, String>,
    args: Vec<String>,
    timeout: Duration,
) -> Result<Vec<u8>, AdapterInstallError> {
    let program = resolve_configured_command(&launch.npm, env);
    let mut command = openbitfun_core::util::process_manager::create_tokio_command(&program);
    command.args(args).kill_on_drop(true);
    apply_command_environment(&mut command, Some(env));
    match tokio::time::timeout(timeout, command.output()).await {
        Ok(Ok(output)) if output.status.success() => Ok(output.stdout),
        Ok(Ok(output)) => Err(AdapterInstallError::Failed(command_error_summary(
            &output.stderr,
            &output.stdout,
        ))),
        Ok(Err(error)) => Err(AdapterInstallError::Failed(error.to_string())),
        Err(_) => Err(AdapterInstallError::TimedOut),
    }
}

/// `npm view <spec> version` prints a bare version for a tag, and one
/// `<package>@<version> '<version>'` line per match for a range, highest last.
fn parse_view_version(stdout: &str) -> Option<String> {
    let line = stdout
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())?;
    let version = match line.rsplit_once('\'') {
        Some((head, _)) => head.rsplit_once('\'').map_or(head, |(_, version)| version),
        None => line,
    };
    Some(version.trim().to_string()).filter(|version| !version.is_empty())
}

/// What a remote store script does when the entry already has an install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteStoreMode {
    /// Use it.
    Ensure,
    /// Ask the registry first, and install a newer release if there is one.
    Update,
}

impl AdapterStore {
    /// The SSH-host counterpart of [`AdapterStore::ensure_installed`].
    pub(crate) async fn ensure_installed_remote(
        &self,
        ssh_manager: &SSHConnectionManager,
        connection_id: &str,
        launch: &AdapterLaunch,
        env: &HashMap<String, String>,
    ) -> Result<InstalledAdapter, AdapterInstallError> {
        let lock = self.entry_lock(&remote_entry_key(connection_id, launch));
        let _guard = lock.lock().await;
        run_remote_store_script(
            ssh_manager,
            connection_id,
            &remote_store_script(launch, env, RemoteStoreMode::Ensure),
            ADAPTER_DOWNLOAD_TIMEOUT,
        )
        .await
    }

    /// The SSH-host counterpart of [`AdapterStore::update_in_background`].
    pub(crate) fn update_remote_in_background(
        self: &Arc<Self>,
        ssh_manager: SSHConnectionManager,
        connection_id: &str,
        client_id: &str,
        launch: AdapterLaunch,
        env: HashMap<String, String>,
    ) {
        let key = remote_entry_key(connection_id, &launch);
        if !launch.follows_releases() || !self.claim_update_check(&key) {
            return;
        }
        let store = Arc::clone(self);
        let connection_id = connection_id.to_string();
        let client_id = client_id.to_string();
        tokio::spawn(async move {
            let lock = store.entry_lock(&key);
            let _guard = lock.lock().await;
            let result = run_remote_store_script(
                &ssh_manager,
                &connection_id,
                &remote_store_script(&launch, &env, RemoteStoreMode::Update),
                ADAPTER_DOWNLOAD_TIMEOUT + UPDATE_LOOKUP_TIMEOUT,
            )
            .await;
            match result {
                Ok(installed) => debug!(
                    "Remote ACP adapter release check finished: id={} spec={} version={:?}",
                    client_id, launch.spec, installed.version
                ),
                Err(error) => warn!(
                    "Remote ACP adapter release check failed: id={} spec={} error={:?}",
                    client_id, launch.spec, error
                ),
            }
        });
    }
}

fn remote_entry_key(connection_id: &str, launch: &AdapterLaunch) -> String {
    format!("ssh:{connection_id}:{}", launch.store_key())
}

async fn run_remote_store_script(
    ssh_manager: &SSHConnectionManager,
    connection_id: &str,
    script: &str,
    timeout: Duration,
) -> Result<InstalledAdapter, AdapterInstallError> {
    let timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
    let output = ssh_manager
        .execute_command_with_options(
            connection_id,
            &remote_user_shell_command(script),
            SSHCommandOptions {
                timeout_ms: Some(timeout_ms),
                cancellation_token: None,
            },
        )
        .await
        .map_err(|error| AdapterInstallError::Failed(error.to_string()))?;
    if output.timed_out {
        return Err(AdapterInstallError::TimedOut);
    }
    if output.interrupted {
        return Err(AdapterInstallError::Failed(
            "command was cancelled".to_string(),
        ));
    }
    if output.exit_code != 0 {
        let summary = remote_command_error_summary(&output.stderr, &output.stdout);
        return Err(AdapterInstallError::Failed(if summary.is_empty() {
            format!("store script exited with {}", output.exit_code)
        } else {
            summary
        }));
    }
    parse_remote_store_output(&output.stdout).ok_or_else(|| {
        AdapterInstallError::Failed("store script did not report the adapter".to_string())
    })
}

/// The script prints the adapter path and then its version, one per line.
fn parse_remote_store_output(stdout: &str) -> Option<InstalledAdapter> {
    let mut lines = stdout.lines().map(str::trim);
    let bin = lines.next().filter(|bin| !bin.is_empty())?.to_string();
    let version = lines
        .next()
        .filter(|version| !version.is_empty())
        .map(str::to_string);
    Some(InstalledAdapter { bin, version })
}

/// The store's local install procedure as a POSIX shell script, so an SSH host
/// needs nothing but npm and node, which the adapter needs anyway.
fn remote_store_script(
    launch: &AdapterLaunch,
    env: &HashMap<String, String>,
    mode: RemoteStoreMode,
) -> String {
    let exports = render_remote_env_assignments(env)
        .into_iter()
        .map(|assignment| format!("export {assignment}\n"))
        .collect::<String>();
    let npm = std::iter::once(shell_escape(&launch.npm))
        .chain(launch.npm_options.iter().map(|option| shell_escape(option)))
        .collect::<Vec<_>>()
        .join(" ");
    let spec = shell_escape(&launch.spec);
    let flags = INSTALL_FLAGS.join(" ");
    let existing = match mode {
        RemoteStoreMode::Ensure => "  report \"$root/$current\"\n  exit 0\n".to_string(),
        RemoteStoreMode::Update => format!(
            "  latest=$({npm} view {spec} version 2>/dev/null | tail -n 1 | sed \"s/.*'\\(.*\\)'.*/\\1/\")\n  \
             if [ -z \"$latest\" ] || [ \"$latest\" = \"$(installed_version \"$root/$current\")\" ]; then\n    \
             report \"$root/$current\"\n    exit 0\n  fi\n"
        ),
    };
    format!(
        "{exports}root=\"{REMOTE_STORE_ROOT}/{key}\"\n\
         bin_path=node_modules/.bin/{bin}\n\
         manifest=node_modules/{package}/package.json\n\
         installed_version() {{ sed -n 's/.*\"version\" *: *\"\\([^\"]*\\)\".*/\\1/p' \"$1/$manifest\" 2>/dev/null | head -n 1; }}\n\
         report() {{ printf '%s\\n%s\\n' \"$1/$bin_path\" \"$(installed_version \"$1\")\"; }}\n\
         current=$(cat \"$root/{CURRENT_FILE}\" 2>/dev/null)\n\
         if [ -n \"$current\" ] && [ -x \"$root/$current/$bin_path\" ]; then\n\
         {existing}\
         fi\n\
         name=\"{INSTALL_DIR_PREFIX}$(date +%s)-$$\"\n\
         mkdir -p \"$root/$name\" || exit 1\n\
         printf '{{\"private\":true}}\\n' > \"$root/$name/package.json\" || exit 1\n\
         if ! {npm} install --prefix \"$root/$name\" {flags} {spec} >&2 || [ ! -x \"$root/$name/$bin_path\" ]; then\n  \
         rm -rf \"$root/$name\"\n  exit 1\n\
         fi\n\
         version=$(installed_version \"$root/$name\")\n\
         if [ -n \"$version\" ] && [ -n \"$current\" ] && [ -x \"$root/$current/$bin_path\" ] && \
         [ \"$version\" = \"$(installed_version \"$root/$current\")\" ]; then\n  \
         rm -rf \"$root/$name\"\n  report \"$root/$current\"\n  exit 0\n\
         fi\n\
         printf '%s\\n' \"$name\" > \"$root/{CURRENT_FILE}.$$\" && mv -f \"$root/{CURRENT_FILE}.$$\" \"$root/{CURRENT_FILE}\" || exit 1\n\
         for old in \"$root\"/{INSTALL_DIR_PREFIX}*; do\n  \
         base=${{old##*/}}\n  \
         if [ \"$base\" != \"$name\" ] && [ \"$base\" != \"$current\" ] && [ -x \"$old/$bin_path\" ]; then rm -rf \"$old\"; fi\n\
         done\n\
         report \"$root/$name\"\n",
        key = launch.store_key(),
        bin = shell_escape(&launch.bin),
        package = shell_escape(&launch.package),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::builtin_clients::default_config_for_builtin_client;
    use crate::client::requirements::acp_requirement_spec;

    fn claude_adapter() -> AcpAdapterSpec<'static> {
        acp_requirement_spec("claude-code", None)
            .adapter
            .expect("Claude adapter")
    }

    fn npx_config(command: &str, args: &[&str]) -> AcpClientConfig {
        let mut config = default_config_for_builtin_client("claude-code").expect("builtin config");
        config.command = command.to_string();
        config.args = args.iter().map(|arg| (*arg).to_string()).collect();
        config
    }

    fn scratch_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "openbitfun-acp-store-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).expect("scratch directory should be created");
        path
    }

    #[test]
    fn builtin_preset_launch_runs_the_installed_adapter_directly() {
        let config = default_config_for_builtin_client("claude-code").expect("builtin config");
        let launch = npx_adapter_launch(&config, &claude_adapter()).expect("npx launch");

        assert_eq!(
            launch.spec(),
            "@agentclientprotocol/claude-agent-acp@latest"
        );
        assert_eq!(launch.npm, "npm");
        assert!(launch.follows_releases());
        assert_eq!(
            launch.store_key(),
            "agentclientprotocol+claude-agent-acp@latest"
        );
        assert_eq!(
            launch.install_args("/store/install-1"),
            [
                "install",
                "--prefix",
                "/store/install-1",
                "--no-audit",
                "--no-fund",
                "--omit=dev",
                "@agentclientprotocol/claude-agent-acp@latest",
            ]
            .map(str::to_string)
        );

        let installed = launch.installed_launch(
            &config,
            "/store/install-1/node_modules/.bin/claude-agent-acp",
        );
        assert_eq!(
            installed.command,
            "/store/install-1/node_modules/.bin/claude-agent-acp"
        );
        assert!(installed.args.is_empty());
        assert_eq!(installed.env, config.env);
        assert_eq!(installed.permission_mode, config.permission_mode);
    }

    #[test]
    fn launch_keeps_npm_options_pinned_versions_and_adapter_args() {
        let config = npx_config(
            "/opt/homebrew/bin/npx",
            &[
                "-y",
                "--registry=https://registry.example.test/",
                "@agentclientprotocol/claude-agent-acp@0.99.0",
                "--adapter-flag",
            ],
        );
        let launch = npx_adapter_launch(&config, &claude_adapter()).expect("npx launch");

        assert_eq!(launch.npm, "/opt/homebrew/bin/npm");
        assert!(!launch.follows_releases());
        assert_eq!(
            launch.view_args(),
            [
                "--registry=https://registry.example.test/",
                "view",
                "@agentclientprotocol/claude-agent-acp@0.99.0",
                "version",
            ]
            .map(str::to_string)
        );
        assert_eq!(
            launch.installed_launch(&config, "/bin/adapter").args,
            vec!["--adapter-flag"]
        );
    }

    #[test]
    fn launch_matches_bare_specs_and_windows_shims() {
        let config = npx_config(
            r"C:\Program Files\nodejs\npx.cmd",
            &["@agentclientprotocol/claude-agent-acp"],
        );
        let launch = npx_adapter_launch(&config, &claude_adapter()).expect("npx launch");

        assert_eq!(launch.npm, r"C:\Program Files\nodejs\npm.cmd");
        assert!(launch.follows_releases());
        assert!(npx_adapter_launch(
            &npx_config("npx", &["@agentclientprotocol/claude-agent-acp@^0.5"]),
            &claude_adapter()
        )
        .expect("range launch")
        .follows_releases());
    }

    #[test]
    fn launch_ignores_commands_that_npx_does_not_install() {
        let adapter = claude_adapter();
        for config in [
            npx_config("claude-agent-acp", &[]),
            npx_config("npxx", &["@agentclientprotocol/claude-agent-acp@latest"]),
            npx_config(
                "npm",
                &["exec", "@agentclientprotocol/claude-agent-acp@latest"],
            ),
            npx_config(
                "npx",
                &["--yes", "@agentclientprotocol/claude-agent-acp-fork@latest"],
            ),
            npx_config("npx", &["--yes", "./local-adapter"]),
            npx_config(
                "npx",
                &[
                    "-p",
                    "other",
                    "@agentclientprotocol/claude-agent-acp@latest",
                ],
            ),
        ] {
            assert_eq!(npx_adapter_launch(&config, &adapter), None, "{config:?}");
        }
    }

    #[test]
    fn view_output_yields_the_highest_version() {
        assert_eq!(parse_view_version("0.6.2\n").as_deref(), Some("0.6.2"));
        assert_eq!(
            parse_view_version("pkg@0.5.0 '0.5.0'\npkg@0.5.3 '0.5.3'\n").as_deref(),
            Some("0.5.3")
        );
        assert_eq!(parse_view_version("\n"), None);
    }

    #[test]
    fn install_failures_name_the_spec_and_the_timeout() {
        let error = AdapterInstallError::TimedOut
            .into_error(
                "claude-code",
                "@agentclientprotocol/claude-agent-acp@latest",
            )
            .to_string();
        assert!(error.contains("@agentclientprotocol/claude-agent-acp@latest"));
        assert!(error.contains(&format!("{}s", ADAPTER_DOWNLOAD_TIMEOUT.as_secs())));
    }

    #[test]
    fn remote_output_reports_the_adapter_and_its_version() {
        assert_eq!(
            parse_remote_store_output("/home/u/a/node_modules/.bin/claude-agent-acp\n0.6.2\n"),
            Some(InstalledAdapter {
                bin: "/home/u/a/node_modules/.bin/claude-agent-acp".to_string(),
                version: Some("0.6.2".to_string()),
            })
        );
        assert_eq!(
            parse_remote_store_output("/a/claude-agent-acp\n\n")
                .and_then(|installed| installed.version),
            None
        );
        assert_eq!(parse_remote_store_output(""), None);
    }

    /// An npm stand-in: `install` lays out what npm would for the adapter, at
    /// the version in `FAKE_ADAPTER_VERSION`; `view` reports
    /// `FAKE_LATEST_VERSION`; `FAKE_NPM_FAIL` makes either fail.
    #[cfg(unix)]
    fn write_fake_npm(directory: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let script = r#"#!/bin/sh
[ -n "$FAKE_NPM_FAIL" ] && { echo "npm ERR! offline" >&2; exit 1; }
prefix=""
while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) prefix="$2"; shift ;;
    view) echo "$FAKE_LATEST_VERSION"; exit 0 ;;
  esac
  shift
done
package="$prefix/node_modules/@agentclientprotocol/claude-agent-acp"
mkdir -p "$prefix/node_modules/.bin" "$package"
printf '{"version":"%s"}\n' "$FAKE_ADAPTER_VERSION" > "$package/package.json"
printf '#!/bin/sh\n' > "$package/cli.js"
chmod +x "$package/cli.js"
ln -s ../@agentclientprotocol/claude-agent-acp/cli.js "$prefix/node_modules/.bin/claude-agent-acp"
"#;
        let path = directory.join("npm");
        std::fs::write(&path, script).expect("fake npm should be written");
        let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("fake npm should be executable");
    }

    #[cfg(unix)]
    fn fake_npm_env(bin_directory: &Path, adapter: &str, latest: &str) -> HashMap<String, String> {
        HashMap::from([
            (
                "PATH".to_string(),
                format!("{}:/usr/bin:/bin", bin_directory.display()),
            ),
            ("FAKE_ADAPTER_VERSION".to_string(), adapter.to_string()),
            ("FAKE_LATEST_VERSION".to_string(), latest.to_string()),
        ])
    }

    #[cfg(unix)]
    fn install_dirs(entry: &Path) -> Vec<String> {
        let mut names = std::fs::read_dir(entry)
            .expect("entry should exist")
            .map(|item| {
                item.expect("entry item")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name.starts_with(INSTALL_DIR_PREFIX))
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[cfg(unix)]
    #[test]
    fn store_installs_once_then_follows_new_releases() {
        let scratch = scratch_directory("local");
        let bin_directory = scratch.join("bin");
        std::fs::create_dir_all(&bin_directory).expect("bin directory");
        write_fake_npm(&bin_directory);
        let store = AdapterStore::new(scratch.join("store"));
        let config = default_config_for_builtin_client("claude-code").expect("builtin config");
        let launch = npx_adapter_launch(&config, &claude_adapter()).expect("npx launch");
        let entry = scratch.join("store").join(launch.store_key());
        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

        assert_eq!(store.installed(&launch), None);
        let first = runtime
            .block_on(
                store.ensure_installed(&launch, &fake_npm_env(&bin_directory, "1.0.0", "1.0.0")),
            )
            .expect("first install");
        assert_eq!(first.version.as_deref(), Some("1.0.0"));
        assert!(Path::new(&first.bin).is_file());
        assert_eq!(store.installed(&launch), Some(first.clone()));

        // Installed already: neither a launch nor an unchanged release reinstalls.
        let failing = HashMap::from([
            ("PATH".to_string(), bin_directory.display().to_string()),
            ("FAKE_NPM_FAIL".to_string(), "1".to_string()),
        ]);
        assert_eq!(
            runtime.block_on(store.ensure_installed(&launch, &failing)),
            Ok(first.clone())
        );
        assert_eq!(
            runtime
                .block_on(store.update(&launch, &fake_npm_env(&bin_directory, "1.0.0", "1.0.0"))),
            Ok(first.clone())
        );
        assert_eq!(install_dirs(&entry).len(), 1);

        // A new release becomes current; the install it replaces stays for the
        // sessions still running it.
        let second = runtime
            .block_on(store.update(&launch, &fake_npm_env(&bin_directory, "1.1.0", "1.1.0")))
            .expect("update");
        assert_eq!(second.version.as_deref(), Some("1.1.0"));
        assert_eq!(store.installed(&launch), Some(second.clone()));
        assert!(Path::new(&first.bin).is_file());

        // The one before that goes.
        let third = runtime
            .block_on(store.update(&launch, &fake_npm_env(&bin_directory, "1.2.0", "1.2.0")))
            .expect("second update");
        assert_eq!(third.version.as_deref(), Some("1.2.0"));
        assert!(!Path::new(&first.bin).exists());
        assert!(Path::new(&second.bin).is_file());
        assert_eq!(install_dirs(&entry).len(), 2);

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[cfg(unix)]
    #[test]
    fn failed_install_leaves_no_install_behind() {
        let scratch = scratch_directory("failed");
        let bin_directory = scratch.join("bin");
        std::fs::create_dir_all(&bin_directory).expect("bin directory");
        write_fake_npm(&bin_directory);
        let store = AdapterStore::new(scratch.join("store"));
        let config = default_config_for_builtin_client("claude-code").expect("builtin config");
        let launch = npx_adapter_launch(&config, &claude_adapter()).expect("npx launch");
        let mut env = fake_npm_env(&bin_directory, "1.0.0", "1.0.0");
        env.insert("FAKE_NPM_FAIL".to_string(), "1".to_string());
        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

        let error = runtime
            .block_on(store.ensure_installed(&launch, &env))
            .expect_err("install fails");
        assert!(
            matches!(&error, AdapterInstallError::Failed(detail) if detail.contains("offline"))
        );
        assert_eq!(store.installed(&launch), None);
        assert!(install_dirs(&scratch.join("store").join(launch.store_key())).is_empty());

        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// Runs the remote store script under a local shell with `HOME` in a
    /// scratch directory: the SSH host path, minus SSH.
    #[cfg(unix)]
    fn run_remote_script(
        home: &Path,
        launch: &AdapterLaunch,
        env: &HashMap<String, String>,
        mode: RemoteStoreMode,
    ) -> Option<InstalledAdapter> {
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(remote_store_script(launch, env, mode))
            .env("HOME", home)
            .output()
            .expect("sh should run");
        assert!(
            output.status.success(),
            "store script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        parse_remote_store_output(&String::from_utf8_lossy(&output.stdout))
    }

    #[cfg(unix)]
    #[test]
    fn remote_store_script_installs_once_then_follows_new_releases() {
        let home = scratch_directory("remote");
        let bin_directory = home.join("bin");
        std::fs::create_dir_all(&bin_directory).expect("bin directory");
        write_fake_npm(&bin_directory);
        let config = default_config_for_builtin_client("claude-code").expect("builtin config");
        let launch = npx_adapter_launch(&config, &claude_adapter()).expect("npx launch");
        let entry = home
            .join(".openbitfun/acp-adapters")
            .join(launch.store_key());

        let first = run_remote_script(
            &home,
            &launch,
            &fake_npm_env(&bin_directory, "1.0.0", "1.0.0"),
            RemoteStoreMode::Ensure,
        )
        .expect("first install");
        assert!(first.bin.starts_with(&entry.display().to_string()));
        assert!(Path::new(&first.bin).is_file());

        let again = run_remote_script(
            &home,
            &launch,
            &fake_npm_env(&bin_directory, "2.0.0", "2.0.0"),
            RemoteStoreMode::Ensure,
        )
        .expect("ensure");
        assert_eq!(again.bin, first.bin, "ensure reuses the current install");

        let updated = run_remote_script(
            &home,
            &launch,
            &fake_npm_env(&bin_directory, "2.0.0", "2.0.0"),
            RemoteStoreMode::Update,
        )
        .expect("update");
        assert_ne!(updated.bin, first.bin);
        assert!(Path::new(&first.bin).is_file(), "previous install is kept");
        assert_eq!(install_dirs(&entry).len(), 2);

        let _ = std::fs::remove_dir_all(&home);
    }
}
