//! Coordination layer
//!
//! Top-level component that integrates all subsystems

mod background_outcomes;
mod control_conversation;
mod coordination_store;
mod delegation_context;
mod external_agent_turn;
mod remote_agent_turn;
pub use control_conversation::{
    ControlConversation, CreateControlConversationRequest, VoiceExchangeRequest,
};
pub mod coordinator;
pub mod scheduler;
pub mod state_manager;
pub mod turn_outcome;
mod turn_settlement;

pub use coordinator::*;
pub use scheduler::*;
pub use state_manager::*;
pub use turn_outcome::*;

pub(crate) use background_outcomes::{
    BackgroundSubagentOutcome, BackgroundSubagentOutcomeStore, BackgroundSubagentWaitMode,
    BackgroundSubagentWaitResult, BackgroundSubagentWaitStatus,
};
pub(crate) use coordination_store::{validate_agent_id, DirectChildAgentRecord};
pub(crate) use external_agent_turn::external_agent_client_id;

/// Whether `agent_type` names an external (`acp:`) agent.
pub fn is_external_agent_type(agent_type: &str) -> bool {
    external_agent_client_id(agent_type).is_some()
}

pub use coordinator::get_global_coordinator;
pub use scheduler::get_global_scheduler;
