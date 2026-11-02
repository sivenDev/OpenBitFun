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
    assert!(!delete_tenant(&path_manager, "workspace", &revoked_with_session).unwrap());

    // Without readable shares nothing is pruned.
    std::fs::write(store.path(), b"{ not json").unwrap();
    store.revoke("workspace", &key(1)).await.unwrap_err();
    assert!(prune_tenants(&path_manager, &store).await.is_err());
    assert!(granted_dir.exists());
}
