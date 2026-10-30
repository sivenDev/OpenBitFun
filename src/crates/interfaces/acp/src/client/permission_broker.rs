//! Routes permission prompts of delegated ACP turns to the OpenBitFun
//! session that owns them, so any surface driving that session can answer.

use agent_client_protocol::schema::{
    PermissionOptionKind, RequestPermissionRequest, RequestPermissionResponse, ToolKind,
};
use openbitfun_core::runtime_ports::{
    ExternalAgentPermissionDecision, ExternalAgentPermissionKind, ExternalAgentPermissionRequest,
};
use tokio::sync::{mpsc, oneshot};

/// One prompt handed to the turn that owns the ACP session. The turn replies
/// `None` when it cannot route prompts, so the standalone prompt is used.
pub(crate) struct BrokeredPermission {
    pub(crate) request: ExternalAgentPermissionRequest,
    pub(crate) reply: oneshot::Sender<Option<ExternalAgentPermissionDecision>>,
}

pub(crate) type PermissionBroker = mpsc::UnboundedSender<BrokeredPermission>;

pub(crate) fn external_permission_request(
    request: &RequestPermissionRequest,
) -> ExternalAgentPermissionRequest {
    let fields = &request.tool_call.fields;
    let kind = match fields.kind {
        Some(ToolKind::Read | ToolKind::Search) => ExternalAgentPermissionKind::Read,
        Some(ToolKind::Edit | ToolKind::Delete | ToolKind::Move) => {
            ExternalAgentPermissionKind::Edit
        }
        Some(ToolKind::Execute) => ExternalAgentPermissionKind::Execute,
        Some(ToolKind::Fetch) => ExternalAgentPermissionKind::Fetch,
        _ => ExternalAgentPermissionKind::Other,
    };
    let resources: Vec<String> = fields
        .locations
        .iter()
        .flatten()
        .map(|location| location.path.to_string_lossy().into_owned())
        .collect();
    let command = fields
        .raw_input
        .as_ref()
        .and_then(|input| input.get("command"))
        .and_then(serde_json::Value::as_str)
        .filter(|command| !command.trim().is_empty())
        .map(str::to_string);
    let tool_call_id = request.tool_call.tool_call_id.to_string();
    ExternalAgentPermissionRequest {
        title: fields
            .title
            .clone()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| tool_call_id.clone()),
        tool_call_id: (!tool_call_id.is_empty()).then_some(tool_call_id),
        kind,
        resources,
        command,
    }
}

/// Picks the ACP option matching `decision`. A cancelled prompt is answered
/// as cancelled so the agent stops waiting without treating it as approval.
pub(crate) fn response_for_decision(
    request: &RequestPermissionRequest,
    decision: ExternalAgentPermissionDecision,
) -> RequestPermissionResponse {
    match decision {
        ExternalAgentPermissionDecision::AllowOnce => super::manager::select_permission_by_kind(
            request,
            PermissionOptionKind::AllowOnce,
            true,
        ),
        ExternalAgentPermissionDecision::AllowAlways => super::manager::select_permission_by_kind(
            request,
            PermissionOptionKind::AllowAlways,
            true,
        ),
        ExternalAgentPermissionDecision::Reject => super::manager::select_permission_by_kind(
            request,
            PermissionOptionKind::RejectOnce,
            false,
        ),
        ExternalAgentPermissionDecision::Cancelled => RequestPermissionResponse::new(
            agent_client_protocol::schema::RequestPermissionOutcome::Cancelled,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::{
        PermissionOption, RequestPermissionOutcome, SessionId, ToolCallLocation, ToolCallUpdate,
        ToolCallUpdateFields,
    };

    fn request(fields: ToolCallUpdateFields) -> RequestPermissionRequest {
        RequestPermissionRequest::new(
            SessionId::new("acp-session-1".to_string()),
            ToolCallUpdate::new("tool-1".to_string(), fields),
            vec![
                PermissionOption::new("deny", "Deny", PermissionOptionKind::RejectOnce),
                PermissionOption::new("once", "Allow", PermissionOptionKind::AllowOnce),
                PermissionOption::new("always", "Always", PermissionOptionKind::AllowAlways),
            ],
        )
    }

    fn selected(response: RequestPermissionResponse) -> Option<String> {
        match response.outcome {
            RequestPermissionOutcome::Selected(selected) => Some(selected.option_id.to_string()),
            _ => None,
        }
    }

    #[test]
    fn prompts_carry_kind_title_paths_and_command() {
        let edit = external_permission_request(&request(
            ToolCallUpdateFields::new()
                .kind(ToolKind::Edit)
                .title("Edit src/main.rs".to_string())
                .locations(vec![ToolCallLocation::new("/home/dev/project/src/main.rs")]),
        ));
        assert_eq!(edit.kind, ExternalAgentPermissionKind::Edit);
        assert_eq!(edit.title, "Edit src/main.rs");
        assert_eq!(edit.tool_call_id.as_deref(), Some("tool-1"));
        assert_eq!(edit.resources, vec!["/home/dev/project/src/main.rs"]);

        let execute = external_permission_request(&request(
            ToolCallUpdateFields::new()
                .kind(ToolKind::Execute)
                .raw_input(serde_json::json!({ "command": "npm test" })),
        ));
        assert_eq!(execute.kind, ExternalAgentPermissionKind::Execute);
        assert_eq!(execute.title, "tool-1");
        assert!(execute.resources.is_empty());
        assert_eq!(execute.command.as_deref(), Some("npm test"));

        let other = external_permission_request(&request(
            ToolCallUpdateFields::new().kind(ToolKind::SwitchMode),
        ));
        assert_eq!(other.kind, ExternalAgentPermissionKind::Other);
    }

    #[test]
    fn decisions_select_the_matching_acp_option() {
        let request = request(ToolCallUpdateFields::new());
        assert_eq!(
            selected(response_for_decision(
                &request,
                ExternalAgentPermissionDecision::AllowOnce
            ))
            .as_deref(),
            Some("once")
        );
        assert_eq!(
            selected(response_for_decision(
                &request,
                ExternalAgentPermissionDecision::AllowAlways
            ))
            .as_deref(),
            Some("always")
        );
        assert_eq!(
            selected(response_for_decision(
                &request,
                ExternalAgentPermissionDecision::Reject
            ))
            .as_deref(),
            Some("deny")
        );
        assert_eq!(
            selected(response_for_decision(
                &request,
                ExternalAgentPermissionDecision::Cancelled
            )),
            None
        );
    }
}
