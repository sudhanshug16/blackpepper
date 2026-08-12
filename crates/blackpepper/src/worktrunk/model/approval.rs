use serde::{Deserialize, Serialize};

use super::APPROVAL_TOKEN_SCHEMA_VERSION;

const MAX_APPROVAL_JSON_BYTES: usize = 1024 * 1024;
const MAX_APPROVAL_COMMANDS: usize = 1024;
const MAX_APPROVAL_PHASE_BYTES: usize = 128;
const MAX_APPROVAL_NAME_BYTES: usize = 512;
const MAX_APPROVAL_TEMPLATE_BYTES: usize = 64 * 1024;

/// Opaque, operation-bound proof of the project-command plan a person saw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktrunkApprovalToken {
    pub schema: u32,
    pub digest: String,
}

impl WorktrunkApprovalToken {
    pub(crate) fn new(digest: String) -> Self {
        Self {
            schema: APPROVAL_TOKEN_SCHEMA_VERSION,
            digest,
        }
    }

    pub(crate) fn is_well_formed(&self) -> bool {
        self.schema == APPROVAL_TOKEN_SCHEMA_VERSION
            && self.digest.len() == 64
            && self.digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    }
}

/// One exact, currently unapproved project command shown before mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktrunkProjectCommand {
    pub phase: String,
    pub name: Option<String>,
    pub template: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorktrunkApprovalState {
    NoCommands,
    ApprovalRequired,
    Approved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorktrunkApprovalPlanCommand {
    pub phase: String,
    pub name: Option<String>,
    pub template: String,
    pub approved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorktrunkApprovalPlan {
    pub state: WorktrunkApprovalState,
    pub commands: Vec<WorktrunkApprovalPlanCommand>,
    pub stale: Vec<String>,
}

impl WorktrunkApprovalPlan {
    pub(crate) fn parse(json: &[u8]) -> Result<Self, String> {
        if json.len() > MAX_APPROVAL_JSON_BYTES {
            return Err("Worktrunk approval JSON exceeded 1 MiB.".to_owned());
        }
        let mut plan: Self = serde_json::from_slice(json)
            .map_err(|error| format!("Worktrunk returned invalid approval JSON: {error}"))?;
        plan.validate()?;
        // Stale approvals have no execution order. Normalizing them avoids a
        // token change caused only by map iteration order in the upstream CLI.
        plan.stale.sort();
        Ok(plan)
    }

    pub(crate) fn unapproved_commands(&self) -> Vec<WorktrunkProjectCommand> {
        self.commands
            .iter()
            .filter(|command| !command.approved)
            .map(|command| WorktrunkProjectCommand {
                phase: command.phase.clone(),
                name: command.name.clone(),
                template: command.template.clone(),
            })
            .collect()
    }

    pub(crate) fn requires_approval(&self) -> bool {
        self.state == WorktrunkApprovalState::ApprovalRequired
    }

    pub(crate) fn is_approved(&self) -> bool {
        self.state == WorktrunkApprovalState::Approved
    }

    pub(crate) fn same_commands_and_stale(&self, other: &Self) -> bool {
        self.commands
            .iter()
            .map(|command| (&command.phase, &command.name, &command.template))
            .eq(other
                .commands
                .iter()
                .map(|command| (&command.phase, &command.name, &command.template)))
            && self.stale == other.stale
    }

    fn validate(&self) -> Result<(), String> {
        if self.commands.len() > MAX_APPROVAL_COMMANDS || self.stale.len() > MAX_APPROVAL_COMMANDS {
            return Err(format!(
                "Worktrunk approval plan exceeds the {MAX_APPROVAL_COMMANDS}-command limit."
            ));
        }
        for command in &self.commands {
            validate_text("phase", &command.phase, MAX_APPROVAL_PHASE_BYTES, false)?;
            if let Some(name) = &command.name {
                validate_text("name", name, MAX_APPROVAL_NAME_BYTES, true)?;
            }
            validate_text(
                "template",
                &command.template,
                MAX_APPROVAL_TEMPLATE_BYTES,
                true,
            )?;
        }
        for stale in &self.stale {
            validate_text("stale template", stale, MAX_APPROVAL_TEMPLATE_BYTES, true)?;
        }
        let expected = if self.commands.is_empty() {
            WorktrunkApprovalState::NoCommands
        } else if self.commands.iter().any(|command| !command.approved) {
            WorktrunkApprovalState::ApprovalRequired
        } else {
            WorktrunkApprovalState::Approved
        };
        if self.state != expected {
            return Err("Worktrunk approval state is inconsistent with its commands.".to_owned());
        }
        Ok(())
    }
}

fn validate_text(
    label: &str,
    value: &str,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<(), String> {
    if (!allow_empty && value.is_empty()) || value.len() > max_bytes || value.contains('\0') {
        return Err(format!(
            "Worktrunk approval {label} is invalid or too long."
        ));
    }
    Ok(())
}
