//! Typed property commands for external agents and operators.

use std::path::Path;

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_markdown::{PropertyEdit, PropertyValue};
use secondbrain_transaction::{
    ApplyPreviewOutcome, PropertyPreview, apply_property_preview, preview_property,
    read_note_properties,
};
use serde::Serialize;

use crate::commands::{CLI_ACTOR, CLI_DEVICE};
use crate::exit::{CliError, OK, read_file};
use crate::output::{Format, Report, emit};

impl Report for PropertyPreview {
    fn render(&self) -> String {
        format!(
            "Previewed property {} for {}\n  {}",
            self.edit.key(),
            self.transaction.path,
            self.transaction.summary
        )
    }
}

#[derive(Serialize)]
struct PropertiesReport {
    path: String,
    properties: std::collections::BTreeMap<String, PropertyValue>,
}

impl Report for PropertiesReport {
    fn render(&self) -> String {
        format!(
            "{}\n  {} editable properties",
            self.path,
            self.properties.len()
        )
    }
}

#[derive(Serialize)]
#[serde(transparent)]
struct ApplyReport(ApplyPreviewOutcome);

impl Report for ApplyReport {
    fn render(&self) -> String {
        format!(
            "{} {}\n  version: {}",
            if self.0.changed {
                "Applied property edit to"
            } else {
                "Property edit changed nothing in"
            },
            self.0.path,
            self.0.version.get()
        )
    }
}

pub fn read(format: Format, workspace: &Path, path: &str) -> Result<u8, CliError> {
    let properties = read_note_properties(workspace, path)?;
    emit(
        format,
        &PropertiesReport {
            path: path.to_owned(),
            properties,
        },
    )?;
    Ok(OK)
}

pub fn preview(
    format: Format,
    workspace: &Path,
    path: &str,
    edit: PropertyEdit,
    out: Option<&Path>,
) -> Result<u8, CliError> {
    let preview = preview_property(
        workspace,
        path,
        edit,
        ActorId::new(CLI_ACTOR)?,
        DeviceId::new(CLI_DEVICE)?,
    )?;
    if let Some(out) = out {
        let encoded = serde_json::to_string_pretty(&preview)?;
        std::fs::write(out, encoded).map_err(|source| CliError::Io {
            operation: "write property preview to",
            path: out.to_path_buf(),
            source,
        })?;
    } else {
        emit(format, &preview)?;
    }
    Ok(OK)
}

pub fn apply(format: Format, workspace: &Path, preview_file: &Path) -> Result<u8, CliError> {
    let preview: PropertyPreview =
        serde_json::from_str(&read_file("read property preview", preview_file)?)
            .map_err(|source| CliError::PlanUnreadable { source })?;
    if preview.transaction.actor.as_str() != CLI_ACTOR
        || preview.transaction.device.as_str() != CLI_DEVICE
    {
        return Err(CliError::ReviewRequired(
            "property preview attribution is not the local CLI".into(),
        ));
    }
    let outcome = apply_property_preview(workspace, &preview)?;
    emit(format, &ApplyReport(outcome))?;
    Ok(OK)
}
