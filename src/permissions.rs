use serde_json::{Value, json};

use crate::cli::{PermissionMode, permission_mode_name};
use crate::error::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationClass {
    Plan,
    LocalRead,
    LocalWrite,
    RemoteRead,
    RemoteWrite,
}

pub fn ensure_permission(
    permission: PermissionMode,
    operation_class: OperationClass,
    operation: &str,
) -> Result<(), Error> {
    let summary = operation_permission_summary(permission, operation, operation_class);
    if summary["agent_may_execute"].as_bool().unwrap_or(false) {
        Ok(())
    } else {
        Err(Error::ApprovalRequired {
            message: format!(
                "approval required for {operation} ({})",
                operation_class_name(operation_class)
            ),
            mode: operation_mode(operation_class),
            stats: json!({
                "permission": summary,
            }),
        })
    }
}

pub fn operation_permission_summary(
    permission: PermissionMode,
    operation: &str,
    operation_class: OperationClass,
) -> Value {
    let mut summary = permission_summary(permission, operation_class);
    summary["operation"] = operation_summary(operation, operation_class);
    summary
}

pub fn operation_summary(operation: &str, operation_class: OperationClass) -> Value {
    json!({
        "name": operation,
        "class": operation_class_name(operation_class),
        "mode": operation_mode(operation_class),
    })
}

pub fn permission_summary(permission: PermissionMode, operation_class: OperationClass) -> Value {
    let agent_may_execute = match permission {
        PermissionMode::Permission => {
            matches!(
                operation_class,
                OperationClass::Plan | OperationClass::LocalRead
            )
        }
        PermissionMode::Auto => matches!(
            operation_class,
            OperationClass::Plan | OperationClass::LocalRead | OperationClass::LocalWrite
        ),
        PermissionMode::Allow => true,
    };
    json!({
        "mode": permission_mode_name(permission),
        "operation_class": operation_class_name(operation_class),
        "approval_required": !agent_may_execute,
        "agent_may_execute": agent_may_execute,
        "reason": permission_reason(permission, operation_class, agent_may_execute),
        "override_hint": if agent_may_execute {
            Value::Null
        } else {
            Value::String(override_hint(operation_class).to_owned())
        },
    })
}

pub const fn operation_class_name(operation_class: OperationClass) -> &'static str {
    match operation_class {
        OperationClass::Plan => "explain",
        OperationClass::LocalRead => "local_read",
        OperationClass::LocalWrite => "local_write",
        OperationClass::RemoteRead => "remote_read",
        OperationClass::RemoteWrite => "remote_write",
    }
}

const fn operation_mode(operation_class: OperationClass) -> &'static str {
    match operation_class {
        OperationClass::Plan | OperationClass::LocalRead | OperationClass::LocalWrite => "local",
        OperationClass::RemoteRead | OperationClass::RemoteWrite => "remote",
    }
}

const fn override_hint(operation_class: OperationClass) -> &'static str {
    match operation_class {
        OperationClass::LocalWrite => "rerun with --permission auto after explicit approval",
        OperationClass::RemoteRead | OperationClass::RemoteWrite => {
            "rerun with --permission allow after explicit approval"
        }
        OperationClass::Plan | OperationClass::LocalRead => "operation requires explicit approval",
    }
}

const fn permission_reason(
    permission: PermissionMode,
    operation_class: OperationClass,
    agent_may_execute: bool,
) -> &'static str {
    match (permission, operation_class, agent_may_execute) {
        (_, _, true) => "operation is allowed by the current permission mode",
        (PermissionMode::Permission, OperationClass::LocalWrite, false) => {
            "permission mode requires approval before local side effects"
        }
        (PermissionMode::Permission | PermissionMode::Auto, OperationClass::RemoteRead, false) => {
            "remote reads can expose credentials, spend money, or move data"
        }
        (PermissionMode::Permission | PermissionMode::Auto, OperationClass::RemoteWrite, false) => {
            "remote writes or mutations require explicit approval"
        }
        _ => "operation requires explicit approval",
    }
}
