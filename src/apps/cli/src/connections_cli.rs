//! `openbitfun connections`: pair this device with a device of another
//! account, and list or revoke those connections. Headless hosts have no
//! settings page, so this is their management entry.
//!
//! Pairing takes three steps: the inviting device runs `invite` and sends the
//! link; the other device runs `accept`, which checks the inviter's key
//! against the link; the inviting device runs `confirm` with the fingerprint
//! the other person reads out from their `connections list`.

use anyhow::{anyhow, bail, Result};
use clap::Subcommand;
use openbitfun_core::service::remote_connect::connection_rpc::{
    authenticated_device_id, ConnectionRpc,
};
use openbitfun_core::service::remote_connect::connections::{
    refusal_reason, ConnectionStore, ConnectionSummary, ConnectionsClient, InviteLink,
};
use openbitfun_core::service::remote_connect::{session_store, AccountClient, AccountSession};

use crate::plugin_diagnostics::escape_terminal_text;

#[derive(Subcommand)]
pub(crate) enum ConnectionsAction {
    /// Create a one-time invite link for a device of another account
    Invite {
        /// Seconds until the invite expires (60 to 604800; default 3600)
        #[arg(long)]
        expires_in: Option<i64>,
        #[arg(long)]
        json: bool,
    },
    /// Accept an invite link after checking who sent it
    Accept {
        link: String,
        /// Accept without showing the inviter first
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        json: bool,
    },
    /// Complete a pairing this device started, after checking the other
    /// device's fingerprint with its owner
    Confirm {
        connection_id: String,
        /// The fingerprint the other person reads from their device
        #[arg(long)]
        fingerprint: String,
    },
    /// List connections, including unreadable entries
    List {
        #[arg(long)]
        json: bool,
    },
    /// Revoke a connection for both sides
    Revoke { connection_id: String },
}

struct SignedIn {
    session: AccountSession,
    relay_url: String,
    device_id: String,
    supported: bool,
}

async fn signed_in() -> Result<SignedIn> {
    let loaded = session_store::load_session_detailed()?
        .ok_or_else(|| anyhow!("not signed in; run `openbitfun login` first"))?;
    let session = AccountSession::new(loaded.token, loaded.user_id, loaded.master_key);
    let device_id = authenticated_device_id(&session)?;
    let capabilities = AccountClient::new()
        .relay_capabilities(&loaded.relay_url)
        .await
        .unwrap_or_default();
    Ok(SignedIn {
        session,
        relay_url: loaded.relay_url,
        device_id,
        supported: ConnectionRpc::relay_supports_connections(&capabilities),
    })
}

fn explain(error: anyhow::Error) -> anyhow::Error {
    let hint = match refusal_reason(&error) {
        Some("fingerprint_mismatch") => {
            "the other device's key does not match the fingerprint; do not continue, and pair again with a new invite"
        }
        Some("store_unreadable") => {
            "this device's connections file cannot be read; it is kept unchanged, repair or move it first"
        }
        Some("different_relay") => "the invite was made on another relay than this device uses",
        Some("invite_not_found") => "the invite does not exist",
        Some("invite_used") => "the invite was already used",
        Some("connection_exists") => "these two devices are already connected",
        Some("inviter_device_required") => "only the inviting device can confirm",
        Some("invite_expired") => "the invite has expired; ask for a new one",
        Some("same_account") => "this device belongs to the same account as the inviter",
        Some("relay_unsupported") => "the relay does not support cross-account connections",
        _ => return error,
    };
    anyhow!("{hint} ({error})")
}

pub(crate) async fn run(action: ConnectionsAction) -> Result<()> {
    let account = signed_in().await?;
    let store = ConnectionStore::open_default()?;
    let client = ConnectionsClient {
        relay_url: &account.relay_url,
        session: &account.session,
        local_device_id: &account.device_id,
        store: &store,
    };
    let needs_relay = !matches!(action, ConnectionsAction::List { .. });
    if needs_relay && !account.supported {
        bail!("the relay does not support cross-account connections (relay_unsupported)");
    }
    match action {
        ConnectionsAction::Invite { expires_in, json } => {
            let invite = client.create_invite(expires_in).await.map_err(explain)?;
            let fingerprint = client.local_fingerprint();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "inviteId": invite.invite_id,
                        "link": invite.link.to_string(),
                        "fingerprint": fingerprint,
                        "expiresAt": invite.expires_at,
                    }))?
                );
            } else {
                println!("Invite link (one use):\n  {}", invite.link);
                println!("This device's fingerprint: {fingerprint}");
                println!(
                    "Send the link to the other person. After they accept, check their fingerprint \
                     and run `openbitfun connections confirm <connection-id> --fingerprint <fp>`."
                );
            }
        }
        ConnectionsAction::Accept { link, yes, json } => {
            let link = InviteLink::parse(&link)?;
            let preview = client.preview(&link).await.map_err(explain)?;
            if !yes {
                println!(
                    "Invite from {} ({}), device {}",
                    escape_terminal_text(&preview.inviter.display_name),
                    escape_terminal_text(&preview.inviter.user_id),
                    escape_terminal_text(&preview.inviter.device_name),
                );
                println!(
                    "Fingerprint (matches the invite link): {}",
                    preview.fingerprint
                );
                println!("Run again with --yes to accept.");
                return Ok(());
            }
            let record = client.accept(&link).await.map_err(explain)?;
            let summary = ConnectionSummary::from(&record);
            if json {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                println!("Connection {} created.", summary.connection_id);
                println!(
                    "Read this device's fingerprint to the inviter: {}",
                    client.local_fingerprint()
                );
                println!("The connection becomes active once they confirm.");
            }
        }
        ConnectionsAction::Confirm {
            connection_id,
            fingerprint,
        } => {
            let record = client
                .confirm(&connection_id, &fingerprint)
                .await
                .map_err(explain)?;
            println!(
                "Connection {} with {} is {}.",
                record.connection_id,
                escape_terminal_text(&record.peer_display_name),
                record.status
            );
        }
        ConnectionsAction::List { json } => {
            let overview = client.overview(account.supported).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&overview)?);
                return Ok(());
            }
            if !overview.supported {
                println!("The relay does not support cross-account connections.");
            }
            if let Some(error) = &overview.relay_error {
                println!(
                    "Relay state unavailable ({}); showing local records.",
                    escape_terminal_text(error)
                );
            }
            println!("This device's fingerprint: {}", overview.local_fingerprint);
            if overview.connections.is_empty() && overview.unreadable.is_empty() {
                println!("No connections.");
            }
            for connection in &overview.connections {
                let state = if connection.awaiting_confirmation {
                    "awaiting your confirmation".to_owned()
                } else if let Some(reason) = &connection.unusable_reason {
                    format!("{} ({reason})", connection.status)
                } else {
                    connection.status.clone()
                };
                println!(
                    "{}  {} ({}), device {}  [{}]",
                    connection.connection_id,
                    escape_terminal_text(&connection.peer_display_name),
                    escape_terminal_text(&connection.peer_user_id),
                    escape_terminal_text(&connection.peer_device_name),
                    state,
                );
                if let Some(fingerprint) = &connection.peer_fingerprint {
                    println!("    fingerprint {fingerprint}");
                }
            }
            for entry in &overview.unreadable {
                println!(
                    "{}  unreadable, disabled: {}",
                    entry
                        .connection_id
                        .as_deref()
                        .map(escape_terminal_text)
                        .unwrap_or_else(|| "(connections file)".into()),
                    escape_terminal_text(&entry.error)
                );
            }
            for invite in &overview.invites {
                println!(
                    "invite {} open until {}",
                    invite.invite_id, invite.expires_at
                );
            }
        }
        ConnectionsAction::Revoke { connection_id } => {
            client.revoke(&connection_id).await.map_err(explain)?;
            println!("Connection {connection_id} revoked.");
        }
    }
    Ok(())
}
