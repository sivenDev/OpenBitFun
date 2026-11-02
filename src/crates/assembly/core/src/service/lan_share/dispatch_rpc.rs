//! Dispatch to a shared assistant over the LAN share channel.
//!
//! The Dispatch controller reaches another device through
//! [`DeviceDispatchRpc`]; for an assistant shared on the local network the
//! device is its owner, and the call goes over the channel pinned to the key
//! this device recorded when access was granted. The owner binds each
//! command to this device's key before its executor sees it.

use async_trait::async_trait;
use serde_json::Value;

use super::host::{LanShareHost, ShareDispatchError};
use crate::service::dispatch::{DeviceDispatchRpc, SharedAssistantProblem};

/// [`DeviceDispatchRpc`] over the LAN share host. A call that cannot reach
/// the owner fails with a [`SharedAssistantProblem`] inside `anyhow`, so
/// callers can tell offline, withdrawn and outdated owners apart; the
/// owner's own refusal keeps its message.
#[derive(Clone)]
pub struct LanDispatchRpc {
    host: LanShareHost,
}

impl LanDispatchRpc {
    pub fn new(host: LanShareHost) -> Self {
        Self { host }
    }
}

#[async_trait]
impl DeviceDispatchRpc for LanDispatchRpc {
    async fn invoke(&self, device_id: &str, command: &str, args: Value) -> anyhow::Result<Value> {
        self.host
            .dispatch(device_id, command, args)
            .await
            .map_err(problem)
    }
}

fn problem(error: ShareDispatchError) -> anyhow::Error {
    match error {
        ShareDispatchError::NotShared => SharedAssistantProblem::NotShared.into(),
        ShareDispatchError::Revoked => SharedAssistantProblem::Revoked.into(),
        ShareDispatchError::Offline(reason) => SharedAssistantProblem::Offline(reason).into(),
        ShareDispatchError::NeedsUpdate(reason) => {
            SharedAssistantProblem::NeedsUpdate(reason).into()
        }
        ShareDispatchError::Refused(reason) => anyhow::anyhow!(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_failures_become_the_reasons_a_conversation_cannot_go_on() {
        let code = |error: ShareDispatchError| {
            problem(error)
                .downcast_ref::<SharedAssistantProblem>()
                .map(SharedAssistantProblem::code)
        };
        assert_eq!(code(ShareDispatchError::NotShared), Some("notShared"));
        assert_eq!(code(ShareDispatchError::Revoked), Some("revoked"));
        assert_eq!(
            code(ShareDispatchError::Offline("gone".into())),
            Some("offline")
        );
        assert_eq!(
            code(ShareDispatchError::NeedsUpdate("old".into())),
            Some("needsUpdate")
        );
        let refused = problem(ShareDispatchError::Refused("not accepted".into()));
        assert!(refused.downcast_ref::<SharedAssistantProblem>().is_none());
        assert_eq!(refused.to_string(), "not accepted");
    }
}
