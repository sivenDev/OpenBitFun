#[cfg(feature = "local-storage")]
use std::path::Path;

use super::*;
#[cfg(feature = "local-storage")]
use crate::infrastructure::PathManager;
use crate::service::delegation::DelegationTrust;

/// A sharing key in hex for `seed`; its device id follows from it.
fn key(seed: u8) -> String {
    hex::encode([seed; 32])
}

fn peer(seed: u8, name: &str) -> SharePeer {
    SharePeer::new(&key(seed), "AB23CD", name, &format!("{name}'s Mac")).unwrap()
}

struct Fixture {
    _dir: tempfile::TempDir,
    store: SharedAssistantStore,
    assistants: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let assistants = dir.path().join("personal_assistant");
    for assistant in ["workspace", "workspace-notes"] {
        std::fs::create_dir_all(assistants.join(assistant)).unwrap();
    }
    Fixture {
        store: SharedAssistantStore::new(dir.path().join("shared-assistants.json")),
        assistants,
        _dir: dir,
    }
}

#[tokio::test]
async fn sharing_validates_settings_and_shares_an_assistant_once() {
    let f = fixture();
    assert!(f.store.shares().await.unwrap().is_empty());

    let shared = f
        .store
        .share(
            &f.assistants,
            "workspace",
            ShareSettings::default(),
            ShareConfirmations::default(),
        )
        .await
        .unwrap();
    assert_eq!(shared.settings.trust, DelegationTrust::Normal);
    assert_eq!(shared.settings.session_retention_days, 7);
    assert_eq!(shared.settings.max_concurrent_tasks, 2);
    assert_eq!(shared.agent_type(), "assistant:workspace");
    assert!(!shared.external_agent_usage_confirmed);

    let again = f
        .store
        .share(
            &f.assistants,
            "workspace",
            ShareSettings::default(),
            ShareConfirmations::default(),
        )
        .await;
    assert!(again.unwrap_err().to_string().contains("already shared"));

    for (settings, problem) in [
        (
            ShareSettings {
                session_retention_days: 3,
                ..ShareSettings::default()
            },
            "kept for one of",
        ),
        (
            ShareSettings {
                max_concurrent_tasks: 0,
                ..ShareSettings::default()
            },
            "concurrent tasks",
        ),
        (
            ShareSettings {
                max_concurrent_tasks: 101,
                ..ShareSettings::default()
            },
            "concurrent tasks",
        ),
    ] {
        let refused = f
            .store
            .share(
                &f.assistants,
                "workspace-notes",
                settings,
                ShareConfirmations::default(),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(refused.contains(problem), "{refused}");
    }

    // Only assistants this device holds are shared.
    for missing in ["workspace-gone", "../workspace", "notes"] {
        assert!(f
            .store
            .share(
                &f.assistants,
                missing,
                ShareSettings::default(),
                ShareConfirmations::default()
            )
            .await
            .is_err());
    }
}

#[tokio::test]
async fn full_trust_needs_its_risk_confirmed_and_usage_confirmation_is_kept() {
    let f = fixture();
    let full = ShareSettings {
        trust: DelegationTrust::Full,
        ..ShareSettings::default()
    };
    let refused = f
        .store
        .share(
            &f.assistants,
            "workspace",
            full,
            ShareConfirmations::default(),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("confirm the risk"), "{refused}");

    f.store
        .share(
            &f.assistants,
            "workspace",
            ShareSettings::default(),
            ShareConfirmations {
                external_agent_usage: true,
                ..ShareConfirmations::default()
            },
        )
        .await
        .unwrap();
    assert!(f
        .store
        .change_settings("workspace", full, ShareConfirmations::default())
        .await
        .is_err());
    let changed = f
        .store
        .change_settings(
            "workspace",
            full,
            ShareConfirmations {
                full_trust_risk: true,
                ..ShareConfirmations::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(changed.settings.trust, DelegationTrust::Full);
    // Once given, the usage confirmation stays with the share.
    assert!(changed.external_agent_usage_confirmed);
    // Staying at full trust needs no second confirmation.
    let kept = f
        .store
        .change_settings(
            "workspace",
            ShareSettings {
                session_retention_days: 30,
                ..full
            },
            ShareConfirmations::default(),
        )
        .await
        .unwrap();
    assert_eq!(kept.settings.session_retention_days, 30);

    let named = f
        .store
        .set_display_name("workspace", " Ann ")
        .await
        .unwrap();
    assert_eq!(named.display_name.as_deref(), Some("Ann"));
    let cleared = f.store.set_display_name("workspace", "  ").await.unwrap();
    assert_eq!(cleared.display_name, None);

    let described = f
        .store
        .set_description(
            "workspace",
            &format!(" Writes\twith care{} ", "!".repeat(300)),
        )
        .await
        .unwrap();
    let description = described.description.unwrap();
    assert!(description.starts_with("Writeswith care!"));
    assert_eq!(description.chars().count(), 200);
    let cleared = f.store.set_description("workspace", " ").await.unwrap();
    assert_eq!(cleared.description, None);
}

#[tokio::test]
async fn access_is_granted_by_public_key_and_ends_with_revoke_block_or_stop() {
    let f = fixture();
    f.store
        .share(
            &f.assistants,
            "workspace",
            ShareSettings::default(),
            ShareConfirmations::default(),
        )
        .await
        .unwrap();
    let bea = peer(1, "Bea");
    assert_eq!(bea.device_id, share_device_id(&key(1)).unwrap());
    assert_eq!(
        f.store.authorize("workspace", &key(1)).await.unwrap(),
        Err(ShareRefusal::NotGranted("workspace".into()))
    );
    assert_eq!(
        f.store.authorize("workspace-notes", &key(1)).await.unwrap(),
        Err(ShareRefusal::NotShared("workspace-notes".into()))
    );

    let grant = f.store.grant("workspace", bea.clone()).await.unwrap();
    assert!(grant.last_used_at.is_none());
    let (share, granted) = f
        .store
        .authorize("workspace", &key(1).to_uppercase())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(share.id, "workspace");
    assert_eq!(granted.peer, bea);
    // Another key is another device, whatever it is called.
    let impostor = SharePeer::new(&key(2), "AB23CD", "Bea", "Bea's Mac").unwrap();
    assert_ne!(impostor.device_id, bea.device_id);
    assert!(f
        .store
        .authorize("workspace", &key(2))
        .await
        .unwrap()
        .is_err());
    // A key that does not match its device id is refused outright.
    let mut forged = bea.clone();
    forged.public_key = key(2);
    assert!(f.store.grant("workspace", forged).await.is_err());

    f.store.record_use("workspace", &key(1)).await.unwrap();
    let used = f.store.share_of("workspace").await.unwrap().unwrap();
    assert!(used.grant(&key(1)).unwrap().last_used_at.is_some());

    assert!(f
        .store
        .revoke("workspace", &key(1))
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        f.store.authorize("workspace", &key(1)).await.unwrap(),
        Err(ShareRefusal::NotGranted("workspace".into()))
    );

    // Blocking withdraws a grant; granting again in person lifts the block.
    f.store.grant("workspace", bea.clone()).await.unwrap();
    f.store.block("workspace", bea.clone()).await.unwrap();
    assert_eq!(
        f.store.authorize("workspace", &key(1)).await.unwrap(),
        Err(ShareRefusal::Blocked("workspace".into()))
    );
    f.store.grant("workspace", bea.clone()).await.unwrap();
    assert!(f
        .store
        .authorize("workspace", &key(1))
        .await
        .unwrap()
        .is_ok());

    // Unblocking lets the device ask again without granting it anything.
    f.store.block("workspace", bea.clone()).await.unwrap();
    assert!(f.store.unblock("workspace", &key(1)).await.unwrap());
    let unblocked = f.store.share_of("workspace").await.unwrap().unwrap();
    assert!(!unblocked.is_blocked(&key(1)));
    assert_eq!(
        f.store.authorize("workspace", &key(1)).await.unwrap(),
        Err(ShareRefusal::NotGranted("workspace".into()))
    );
    assert!(!f.store.unblock("workspace", &key(1)).await.unwrap());
    f.store.grant("workspace", bea.clone()).await.unwrap();

    let stopped = f.store.stop("workspace").await.unwrap().unwrap();
    assert_eq!(stopped.grants.len(), 1);
    assert_eq!(
        f.store.authorize("workspace", &key(1)).await.unwrap(),
        Err(ShareRefusal::NotShared("workspace".into()))
    );
}

#[tokio::test]
async fn an_unreadable_store_is_reported_and_never_overwritten() {
    let f = fixture();
    std::fs::write(f.store.path(), b"{ not json").unwrap();
    assert!(f.store.shares().await.is_err());
    assert!(f.store.shares_now().is_err());
    assert!(f
        .store
        .share(
            &f.assistants,
            "workspace",
            ShareSettings::default(),
            ShareConfirmations::default()
        )
        .await
        .is_err());
    assert_eq!(std::fs::read(f.store.path()).unwrap(), b"{ not json");
}

#[test]
fn peers_need_a_real_key_a_fingerprint_and_names() {
    assert!(SharePeer::new("abcd", "AB23CD", "Bea", "Mac").is_err());
    assert!(SharePeer::new(&key(1), "AB", "Bea", "Mac").is_err());
    assert!(SharePeer::new(&key(1), "AB23CD", " ", "Mac").is_err());
    let long = "x".repeat(200);
    let bounded = SharePeer::new(&key(1), "AB23CD", &long, "Mac").unwrap();
    assert_eq!(bounded.display_name.chars().count(), 64);
}

fn user(name: &str) -> TenantUser {
    TenantUser {
        display_name: name.to_string(),
        device_name: format!("{name}'s Mac"),
    }
}

#[test]
fn each_device_gets_one_workspace_with_its_own_user_profile() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("tenants");
    let bea = share_device_id(&key(1)).unwrap();
    let carl = share_device_id(&key(2)).unwrap();

    let first = ensure_tenant(&root, "workspace", &bea, &user("Bea")).unwrap();
    assert_eq!(first, root.join("workspace").join(&bea));
    let profile = std::fs::read_to_string(first.join("USER.md")).unwrap();
    assert!(
        profile.contains("Bea") && profile.contains("Bea's Mac"),
        "{profile}"
    );
    assert!(!first.join("BOOTSTRAP.md").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&first).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    // The same device returns to the same workspace; its profile is kept.
    std::fs::write(first.join("USER.md"), "Bea likes short answers").unwrap();
    let again = ensure_tenant(&root, "workspace", &bea, &user("Bea")).unwrap();
    assert_eq!(again, first);
    assert_eq!(
        std::fs::read_to_string(first.join("USER.md")).unwrap(),
        "Bea likes short answers"
    );

    // The owner can still name the device once its profile is the user's own.
    let tenant = list_tenants(&root)
        .into_iter()
        .find(|tenant| tenant.path == first)
        .unwrap();
    assert_eq!(tenant_user(&tenant), Some(user("Bea")));

    // Another device, or another assistant, is another workspace.
    let other = ensure_tenant(&root, "workspace", &carl, &user("Carl")).unwrap();
    assert_ne!(other, first);
    let notes = ensure_tenant(&root, "workspace-notes", &bea, &user("Bea")).unwrap();
    assert_ne!(notes, first);
    let listed: Vec<_> = list_tenants(&root)
        .into_iter()
        .map(|tenant| (tenant.assistant_id, tenant.device_id))
        .collect();
    assert_eq!(listed.len(), 3);
    assert!(listed.contains(&("workspace".to_string(), carl.clone())));

    // Paths never come from the caller.
    for (assistant, device) in [
        ("../workspace", bea.as_str()),
        ("workspace", "../../etc"),
        ("workspace", "laptop"),
        ("notes", bea.as_str()),
    ] {
        assert!(
            tenant_dir(&root, assistant, device).is_err(),
            "{assistant} {device}"
        );
    }
}

#[cfg(feature = "local-storage")]
async fn save_session(path_manager: &PathManager, workspace: &Path, session_id: &str) {
    let store = openbitfun_services_core::session::SessionMetadataStore::new(
        path_manager.project_sessions_dir(workspace),
    );
    let metadata = openbitfun_services_core::session::SessionMetadata::new(
        session_id.to_string(),
        "Task".to_string(),
        "assistant:workspace".to_string(),
        "model".to_string(),
    );
    store.save_metadata(&metadata).await.unwrap();
}

#[cfg(feature = "local-storage")]
#[tokio::test]
async fn workspaces_without_a_grant_go_once_their_sessions_are_gone() {
    let dir = tempfile::tempdir().unwrap();
    let path_manager = PathManager::with_user_root_for_tests(dir.path().join("config"));
    let assistants = path_manager.assistant_workspace_base_dir(None);
    std::fs::create_dir_all(assistants.join("workspace")).unwrap();
    let store = SharedAssistantStore::for_product_home(&path_manager);
    store
        .share(
            &assistants,
            "workspace",
            ShareSettings::default(),
            ShareConfirmations::default(),
        )
        .await
        .unwrap();
    let root = tenants_root(&path_manager);
    let [granted, revoked_with_session, revoked_empty] =
        [1u8, 2, 3].map(|seed| share_device_id(&key(seed)).unwrap());
    store.grant("workspace", peer(1, "Bea")).await.unwrap();
    let granted_dir = ensure_tenant(&root, "workspace", &granted, &user("Bea")).unwrap();
    let kept_dir = ensure_tenant(&root, "workspace", &revoked_with_session, &user("Carl")).unwrap();
    save_session(&path_manager, &kept_dir, "delegation-1").await;
    let empty_dir = ensure_tenant(&root, "workspace", &revoked_empty, &user("Dee")).unwrap();
    save_session(&path_manager, &empty_dir, "delegation-2").await;
    let empty_runtime = path_manager.project_runtime_root(&empty_dir);
    // Its last session expired.
    openbitfun_services_core::session::SessionMetadataStore::new(
        path_manager.project_sessions_dir(&empty_dir),
    )
    .delete_session_dir_and_index("delegation-2")
    .await
    .unwrap();

    let deleted = prune_tenants(&path_manager, &store).await.unwrap();
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].device_id, revoked_empty);
    assert!(!empty_dir.exists());
    assert!(!empty_runtime.exists());
    assert!(granted_dir.exists() && kept_dir.exists());

    // Removing a user deletes their workspace at once, sessions and all.
    assert!(delete_tenant(&path_manager, "workspace", &revoked_with_session).unwrap());
    assert!(!kept_dir.exists());
    for device in [&revoked_with_session, &revoked_empty] {
        assert!(!root
            .join("workspace")
            .join(format!("{device}.user.json"))
            .exists());
    }
    assert!(root
        .join("workspace")
        .join(format!("{granted}.user.json"))
        .exists());
    assert!(!delete_tenant(&path_manager, "workspace", &revoked_with_session).unwrap());

    // Without readable shares nothing is pruned.
    std::fs::write(store.path(), b"{ not json").unwrap();
    store.revoke("workspace", &key(1)).await.unwrap_err();
    assert!(prune_tenants(&path_manager, &store).await.is_err());
    assert!(granted_dir.exists());
}

mod clocks {
    use super::super::protocol::{to_local_clock, Access};

    #[test]
    fn times_move_onto_this_devices_clock_and_keep_how_far_ahead_they_lie() {
        // The owner's clock runs 3 s ahead of this device's.
        let owner_now = 1_000_000;
        let local_now = 997_000;
        assert_eq!(to_local_clock(1_060_000, owner_now, local_now), 1_057_000);
        // An owner that did not say what time it was is taken at its word.
        assert_eq!(to_local_clock(1_060_000, 0, local_now), 1_060_000);
        assert_eq!(to_local_clock(10, 1_000_000, 0), 0);

        let cooling = Access::Cooling {
            retry_at_ms: 1_060_000,
        }
        .on_local_clock(owner_now, local_now);
        assert_eq!(
            cooling,
            Access::Cooling {
                retry_at_ms: 1_057_000
            }
        );
        let pending = Access::Pending {
            request_id: "r".into(),
            deadline_ms: 1_060_000,
        }
        .on_local_clock(owner_now, local_now);
        assert_eq!(
            pending,
            Access::Pending {
                request_id: "r".into(),
                deadline_ms: 1_057_000
            }
        );
        assert_eq!(
            Access::Granted.on_local_clock(owner_now, local_now),
            Access::Granted
        );
    }
}

mod request_queue {
    use super::super::protocol::{Access, RequestState};
    use super::super::requests::{Opened, RequestQueue, RequestRefusal};
    use super::{key, peer};

    const WINDOW: u64 = 60_000;
    const COOLDOWN: u64 = 60_000;

    fn pending(opened: Result<Opened, RequestRefusal>) -> String {
        match opened {
            Ok(Opened::Pending(request)) => request.request_id,
            other => panic!("expected an open request, got {other:?}"),
        }
    }

    #[test]
    fn one_open_request_per_device_and_assistant_until_it_runs_out() {
        let mut queue = RequestQueue::new(WINDOW, COOLDOWN);
        let first = pending(queue.open(peer(1, "Ann"), "workspace", false, 1_000));
        // Asking again answers with the open request.
        assert_eq!(
            queue.open(peer(1, "Ann"), "workspace", false, 2_000),
            Err(RequestRefusal::AlreadyPending {
                request_id: first.clone(),
                deadline_ms: 61_000,
            })
        );
        // Another assistant or another device is a separate request.
        pending(queue.open(peer(1, "Ann"), "workspace-notes", false, 2_000));
        pending(queue.open(peer(2, "Bo"), "workspace", false, 2_000));
        assert_eq!(queue.pending().len(), 3);

        assert!(queue.expire(60_999).is_empty());
        let expired = queue.expire(61_000);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].request_id, first);
        assert_eq!(queue.status(&key(1), &first).state, RequestState::Declined);
        assert_eq!(
            queue.access(&key(1), "workspace", 61_000),
            Some(Access::Cooling {
                retry_at_ms: 121_000
            })
        );
        assert_eq!(
            queue.open(peer(1, "Ann"), "workspace", false, 91_000),
            Err(RequestRefusal::Cooling {
                retry_at_ms: 121_000,
                seconds: 30,
            })
        );
        pending(queue.open(peer(1, "Ann"), "workspace", false, 121_000));
    }

    #[test]
    fn a_decline_cools_down_and_an_allow_does_not() {
        let mut queue = RequestQueue::new(WINDOW, COOLDOWN);
        let declined = pending(queue.open(peer(1, "Ann"), "workspace", false, 0));
        let allowed = pending(queue.open(peer(2, "Bo"), "workspace", false, 0));

        assert!(queue.decide(&declined, false, 5_000).is_some());
        assert!(
            queue.decide(&declined, true, 6_000).is_none(),
            "answered once"
        );
        assert!(queue.decide(&allowed, true, 5_000).is_some());

        assert_eq!(queue.status(&key(2), &allowed).state, RequestState::Granted);
        assert_eq!(queue.access(&key(2), "workspace", 5_000), None);
        assert!(matches!(
            queue.open(peer(1, "Ann"), "workspace", false, 6_000),
            Err(RequestRefusal::Cooling { .. })
        ));
        assert!(queue.pending().is_empty());
    }

    #[test]
    fn a_blocked_device_is_declined_without_asking() {
        let mut queue = RequestQueue::new(WINDOW, COOLDOWN);
        let Ok(Opened::Declined { request_id }) = queue.open(peer(1, "Ann"), "workspace", true, 0)
        else {
            panic!("a blocked device's request is declined at once");
        };
        assert!(queue.pending().is_empty());
        assert_eq!(
            queue.status(&key(1), &request_id).state,
            RequestState::Declined
        );
    }

    #[test]
    fn a_device_sees_only_its_own_requests() {
        let mut queue = RequestQueue::new(WINDOW, COOLDOWN);
        let request = pending(queue.open(peer(1, "Ann"), "workspace", false, 0));
        let status = queue.status(&key(1), &request);
        assert_eq!(status.state, RequestState::Pending);
        assert_eq!(status.deadline_ms, Some(60_000));
        assert_eq!(queue.status(&key(2), &request).state, RequestState::Unknown);
        queue.decide(&request, true, 1_000);
        assert_eq!(queue.status(&key(2), &request).state, RequestState::Unknown);
    }

    #[test]
    fn stopping_a_share_ends_its_requests_without_a_cooldown() {
        let mut queue = RequestQueue::new(WINDOW, COOLDOWN);
        let request = pending(queue.open(peer(1, "Ann"), "workspace", false, 0));
        pending(queue.open(peer(1, "Ann"), "workspace-notes", false, 0));
        let withdrawn = queue.withdraw_assistant("workspace", 1_000);
        assert_eq!(withdrawn.len(), 1);
        assert_eq!(queue.pending().len(), 1);
        assert_eq!(
            queue.status(&key(1), &request).state,
            RequestState::Declined
        );
        assert_eq!(queue.access(&key(1), "workspace", 1_000), None);
    }
}

mod shared_with_me_store {
    use super::super::protocol::{Access, CatalogAssistant};
    use super::super::shared_with_me::{SharedWithMeState, SharedWithMeStore};
    use super::{key, peer};

    fn assistant(id: &str, name: &str, access: Access) -> CatalogAssistant {
        CatalogAssistant {
            id: id.to_string(),
            name: name.to_string(),
            description: None,
            access,
        }
    }

    #[tokio::test]
    async fn grants_follow_the_owners_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let store = SharedWithMeStore::new(dir.path().join("shared-with-me.json"));
        let owner = peer(7, "Owner");
        store
            .add_grant(
                owner.clone(),
                &assistant("workspace", "Writer", Access::Granted),
                Some("192.168.1.7:5000".to_string()),
            )
            .await
            .unwrap();
        store
            .add_grant(
                owner.clone(),
                &assistant("workspace-notes", "Notes", Access::Granted),
                None,
            )
            .await
            .unwrap();

        // The owner renamed one and no longer grants the other.
        let changed = store
            .sync_with_catalog(
                &key(7),
                &[
                    assistant("workspace", "Editor", Access::Granted),
                    assistant("workspace-notes", "Notes", Access::Available),
                ],
                Some("192.168.1.8:5000".to_string()),
            )
            .await
            .unwrap();
        assert!(changed);
        let mine = store.assistants().await.unwrap();
        assert_eq!(mine[0].name, "Editor");
        assert_eq!(mine[0].state, SharedWithMeState::Available);
        assert_eq!(mine[0].last_address.as_deref(), Some("192.168.1.8:5000"));
        assert_eq!(mine[1].state, SharedWithMeState::Revoked);

        // A withdrawal pushed by the owner.
        assert!(store.mark_revoked(&key(7), "workspace").await.unwrap());
        assert!(!store.mark_revoked(&key(7), "workspace").await.unwrap());
        // Another owner's key changes nothing.
        assert!(!store
            .mark_revoked(&key(8), "workspace-notes")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn a_removal_is_kept_until_the_owner_hears_of_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = SharedWithMeStore::new(dir.path().join("shared-with-me.json"));
        store
            .add_grant(
                peer(7, "Owner"),
                &assistant("workspace", "Writer", Access::Granted),
                None,
            )
            .await
            .unwrap();
        let removed = store.remove(&key(7), "workspace", true).await.unwrap();
        assert!(removed.is_some());
        assert!(store.assistants().await.unwrap().is_empty());
        assert_eq!(store.pending_removals().await.unwrap().len(), 1);
        assert!(store
            .remove(&key(7), "workspace", true)
            .await
            .unwrap()
            .is_none());

        store.removal_delivered(&key(7), "workspace").await.unwrap();
        assert!(store.pending_removals().await.unwrap().is_empty());

        // A new grant replaces a removal not yet delivered.
        store
            .add_grant(
                peer(7, "Owner"),
                &assistant("workspace", "Writer", Access::Granted),
                None,
            )
            .await
            .unwrap();
        store.remove(&key(7), "workspace", true).await.unwrap();
        store
            .add_grant(
                peer(7, "Owner"),
                &assistant("workspace", "Writer", Access::Granted),
                None,
            )
            .await
            .unwrap();
        assert!(store.pending_removals().await.unwrap().is_empty());
        assert_eq!(store.assistants().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn an_unreadable_file_is_reported_and_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shared-with-me.json");
        std::fs::write(&path, b"{ not json").unwrap();
        let store = SharedWithMeStore::new(path.clone());
        assert!(store.assistants().await.is_err());
        assert!(store
            .add_grant(
                peer(7, "Owner"),
                &assistant("workspace", "Writer", Access::Granted),
                None,
            )
            .await
            .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{ not json");
    }
}

mod added_device_store {
    use super::super::added_devices::AddedDeviceStore;
    use super::{key, peer};

    #[tokio::test]
    async fn devices_are_added_moved_and_forgotten_by_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = AddedDeviceStore::new(dir.path().join("added-devices.json"));
        let owner = peer(7, "Owner");
        store
            .add(owner.clone(), "172.17.9.12:47652".to_string())
            .await
            .unwrap();
        // Adding the same key again moves it rather than listing it twice.
        store
            .add(owner.clone(), "172.17.9.40:47652".to_string())
            .await
            .unwrap();
        let devices = store.devices().await.unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].address, "172.17.9.40:47652");

        assert!(store
            .set_address(
                &key(7).to_ascii_uppercase(),
                "172.17.9.41:47652".to_string()
            )
            .await
            .unwrap());
        assert!(!store
            .set_address(&key(7), "172.17.9.41:47652".to_string())
            .await
            .unwrap());
        assert!(!store
            .set_address(&key(8), "172.17.9.42:47652".to_string())
            .await
            .unwrap());
        assert_eq!(
            store
                .device_with_key(&key(7))
                .await
                .unwrap()
                .unwrap()
                .address,
            "172.17.9.41:47652"
        );

        assert!(store.remove(&owner.device_id).await.unwrap().is_some());
        assert!(store.remove(&owner.device_id).await.unwrap().is_none());
        assert!(store.devices().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_unreadable_file_is_reported_and_left_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("added-devices.json");
        std::fs::write(&path, "not json").unwrap();
        let store = AddedDeviceStore::new(path.clone());
        assert!(store.devices().await.is_err());
        assert!(store
            .add(peer(7, "Owner"), "172.17.9.12:47652".to_string())
            .await
            .is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not json");
    }
}

#[cfg(feature = "lan-share")]
mod browse_state {
    use std::net::{Ipv4Addr, SocketAddr};

    use openbitfun_services_integrations::lan_share::{DiscoveredPeer, PROTOCOL_VERSION};

    use super::super::browse::{BrowseState, DeviceStatus};

    fn device(seed: u8, address: [u8; 4], port: u16) -> DiscoveredPeer {
        let public_key = [seed; 32];
        DiscoveredPeer {
            device_id: super::super::share_device_id(&hex::encode(public_key)).unwrap(),
            device_name: "Owner's Mac".to_string(),
            display_name: "Owner".to_string(),
            public_key,
            fingerprint: "AB23CD".to_string(),
            protocol_version: PROTOCOL_VERSION,
            addresses: vec![Ipv4Addr::from(address)],
            port,
        }
    }

    #[test]
    fn only_devices_that_did_not_answer_are_tried_again() {
        let mut browse = BrowseState::default();
        let away = device(7, [172, 17, 9, 12], 47652);
        let ready = device(8, [172, 17, 9, 13], 47652);
        let changed = device(9, [172, 17, 9, 14], 47652);
        browse.add(away.clone(), "172.17.9.12:47652".to_string());
        browse.add(ready.clone(), "172.17.9.13:47652".to_string());
        browse.add(changed.clone(), "172.17.9.14:47652".to_string());
        assert!(browse.unreachable_peers().is_empty());

        browse.failed(
            &away.device_id,
            DeviceStatus::Unreachable {
                message: "timed out".to_string(),
            },
        );
        browse.catalog(&ready.device_id, Vec::new());
        browse.failed(
            &changed.device_id,
            DeviceStatus::IdentityChanged {
                message: "another key".to_string(),
            },
        );
        let again: Vec<String> = browse
            .unreachable_peers()
            .into_iter()
            .map(|peer| peer.device_id)
            .collect();
        assert_eq!(again, vec![away.device_id.clone()]);

        browse.catalog(&away.device_id, Vec::new());
        assert!(browse.unreachable_peers().is_empty());
    }

    #[test]
    fn a_device_found_and_added_is_listed_once_and_reached_both_ways() {
        let mut browse = BrowseState::default();
        let added = device(7, [172, 17, 9, 12], 47652);
        let found = device(7, [172, 16, 23, 9], 47652);
        let id = added.device_id.clone();

        assert!(browse.add(added.clone(), "172.17.9.12:47652".to_string()));
        browse.found(found.clone());
        let devices = browse.devices();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].status, DeviceStatus::Connecting);
        assert_eq!(
            devices[0].added_address.as_deref(),
            Some("172.17.9.12:47652")
        );
        assert_eq!(browse.readable_peers().len(), 1);
        assert_eq!(
            browse.addresses(&id),
            vec![
                SocketAddr::from(([172, 16, 23, 9], 47652)),
                SocketAddr::from(([172, 17, 9, 12], 47652)),
            ]
        );
        assert_eq!(
            browse.device_with_key(&hex::encode([7; 32])),
            Some(id.clone())
        );

        // Bonjour losing it leaves the added device.
        browse.lost(&id);
        assert_eq!(browse.devices().len(), 1);
        assert_eq!(
            browse.addresses(&id),
            vec![SocketAddr::from(([172, 17, 9, 12], 47652))]
        );

        // Forgetting a device Bonjour lists keeps it, without the address.
        browse.found(found);
        assert!(browse.forget(&id));
        let devices = browse.devices();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].added_address, None);
        assert!(!browse.forget(&id));
        browse.lost(&id);
        assert!(browse.devices().is_empty());

        // Forgetting one only added removes it.
        browse.add(added, "172.17.9.12:47652".to_string());
        assert!(browse.forget(&id));
        assert!(browse.devices().is_empty());
        assert!(browse.addresses(&id).is_empty());
    }
}

#[test]
fn a_worker_reads_the_sharing_id_its_host_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let path_manager = PathManager::with_user_root_for_tests(dir.path().join("config"));
    assert_eq!(local_share_device_id(&path_manager), None);
    let lan_share = path_manager.product_home_dir().join("lan-share");
    let device_id = share_device_id(&key(1)).unwrap();
    super::tenants::record_share_device_id(&lan_share, &device_id).unwrap();
    assert_eq!(local_share_device_id(&path_manager), Some(device_id));
    // Anything else in the file is not an id.
    std::fs::write(lan_share.join("device-id"), "local").unwrap();
    assert_eq!(local_share_device_id(&path_manager), None);
}
