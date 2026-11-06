//! The companion names the UI shows for subagent sessions, resolved here the
//! same way so the model can match a name the user refers to. The pool and its
//! order are a versioned contract with the Web UI resolver
//! (`src/web-ui/src/flow_chat/subagent-identity/nameResolver.ts`); the names
//! themselves are shared terms (`subagentNames.*`).

use crate::service::i18n::generated_locale_contract::generated_shared_term;
use crate::service::i18n::LocaleId;

/// Do not reorder or grow this pool: restored sessions keep their names in
/// every locale.
const SUBAGENT_NAME_KEYS: [&str; 30] = [
    "bouncyBean",
    "cheekyBao",
    "sneakyMochi",
    "zoomingPotato",
    "curiousOtter",
    "discoPenguin",
    "noodleNinja",
    "bubbleBandit",
    "sleepyTaco",
    "pixelRascal",
    "jellyRocket",
    "wobblyPudding",
    "waffleWizard",
    "sneezingDragon",
    "giggleFox",
    "turboSnail",
    "marshmallowPirate",
    "pogoPanda",
    "toastDetective",
    "zigzagDuck",
    "mangoGoblin",
    "biscuitBandit",
    "cometHamster",
    "captainDucky",
    "hiccupSeal",
    "popcornImp",
    "wobbleCorgi",
    "ninjaPeach",
    "cloudHopper",
    "moonBunny",
];

/// FNV-1a over UTF-16 code units, as `hashSubagentIdentity` in the Web UI.
fn identity_hash(value: &str) -> u32 {
    value.encode_utf16().fold(0x811c_9dc5, |hash, unit| {
        (hash ^ u32::from(unit)).wrapping_mul(0x0100_0193)
    })
}

fn subagent_name_key(child_session_id: &str) -> &'static str {
    let seed = child_session_id.trim();
    if seed.is_empty() {
        return SUBAGENT_NAME_KEYS[0];
    }
    let hash = identity_hash(&format!("subagent-names-v1:{seed}"));
    SUBAGENT_NAME_KEYS[hash as usize % SUBAGENT_NAME_KEYS.len()]
}

/// The name the UI shows for the subagent whose session is `child_session_id`.
pub(crate) fn subagent_display_name(
    child_session_id: &str,
    locale: LocaleId,
) -> Option<&'static str> {
    generated_shared_term(
        locale,
        &format!("subagentNames.{}", subagent_name_key(child_session_id)),
    )
}

/// `agent_id`, followed by the name the user sees for it when one exists.
pub(crate) fn describe_agent_for_model(
    agent_id: &str,
    child_session_id: &str,
    locale: LocaleId,
) -> String {
    match subagent_display_name(child_session_id, locale) {
        Some(name) => format!("{agent_id} (shown to the user as \"{name}\")"),
        None => agent_id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The Web UI resolver test pins the same vectors.
    #[test]
    fn picks_the_names_the_web_ui_shows() {
        assert_eq!(
            subagent_name_key("a3ccf64c-8a61-4281-9d87-7fdb78f13fa7"),
            "captainDucky"
        );
        assert_eq!(
            subagent_name_key("875406c8-de4b-4906-a4f7-82c43436a88d"),
            "jellyRocket"
        );
        assert_eq!(subagent_name_key("child-session"), "cloudHopper");
        assert_eq!(subagent_name_key(" child-session "), "cloudHopper");
        assert_eq!(subagent_name_key("   "), SUBAGENT_NAME_KEYS[0]);
        assert_eq!(
            subagent_display_name("a3ccf64c-8a61-4281-9d87-7fdb78f13fa7", LocaleId::ZhCN),
            Some("小黄鸭")
        );
    }

    #[test]
    fn every_pool_entry_is_a_shared_term_in_every_locale() {
        for locale in LocaleId::all() {
            for key in SUBAGENT_NAME_KEYS {
                assert!(
                    generated_shared_term(locale, &format!("subagentNames.{key}")).is_some(),
                    "missing subagentNames.{key} in {}",
                    locale.as_str()
                );
            }
        }
    }
}
