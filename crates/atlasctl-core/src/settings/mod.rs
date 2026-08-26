// SPDX-License-Identifier: AGPL-3.0-only

//! What a remote client may set on a launch, and the check that enforces it.

mod denied;
mod memory;
mod spec;
mod table;
mod values;

pub use spec::{BoundSpec, Disposition, Spec};

/// Every flag and its disposition, in declaration order.
///
/// Three statics rather than one because the exposed half is a UI description
/// and the denied half is a list of refusals, and because one file holding all
/// of it would be over this project's per-file limit. Order is preserved across
/// the join: it is what a form renders, and `denied` contributes nothing to
/// `schema()`, so appending it last changes no output.
pub fn dispositions() -> impl Iterator<Item = &'static (&'static str, Disposition)> {
    table::EXPOSED
        .iter()
        .chain(memory::EXPOSED_MEMORY)
        .chain(denied::DENIED)
}

// `flags` is used by the exhaustiveness tests, which are the reason this
// module and the flag table must stay in step.
#[cfg(test)]
use crate::flags::FlagKind;
use crate::scalar::ScalarValue;
use atlasctl_protocol::settings::{SettingError, SettingSpec, SettingValue};
use std::collections::BTreeMap;

/// The schema, as served to a client at the handshake.
///
/// Sent at runtime rather than baked into the page, because the agent is what
/// validates: a statically-embedded schema is wrong the moment either side
/// ships independently, and a client rendering bounds the validator does not
/// share is how "it looked fine and then the launch was rejected" happens.
pub fn schema() -> Vec<SettingSpec> {
    dispositions()
        .filter_map(|(key, d)| match d {
            Disposition::Expose(s) => Some(SettingSpec {
                key: (*key).to_string(),
                bound: s.bound.to_bound(),
                label: s.label.to_string(),
                help: s.help.to_string(),
                unit: s.unit.map(str::to_string),
                group: s.group,
                advanced: s.advanced,
                locked: false,
            }),
            Disposition::Deny(_) => None,
        })
        .collect()
}

/// Look up a key's disposition.
fn disposition(key: &str) -> Option<&'static Disposition> {
    dispositions().find(|(k, _)| *k == key).map(|(_, d)| d)
}

/// Validate a client's requested overrides.
///
/// Returns values ready for the config chain. Every error is collected rather
/// than returning on the first, so a client can fix a form in one pass instead
/// of discovering problems one at a time.
pub fn validate(
    requested: &BTreeMap<String, SettingValue>,
) -> Result<BTreeMap<String, ScalarValue>, Vec<SettingError>> {
    let mut out = BTreeMap::new();
    let mut errors = Vec::new();

    for (key, value) in requested {
        match disposition(key) {
            None => errors.push(SettingError::UnknownKey { key: key.clone() }),
            // A denied key is a signal, not a typo: nothing in a legitimate UI
            // offers one, so an attempt to set it says something about the
            // client. The caller logs these.
            Some(Disposition::Deny(reason)) => errors.push(SettingError::Denied {
                key: key.clone(),
                reason: (*reason).to_string(),
            }),
            Some(Disposition::Expose(spec)) => match spec.bound.to_bound().check(key, value) {
                Ok(checked) => {
                    out.insert(key.clone(), to_scalar(&checked));
                }
                Err(e) => errors.push(e),
            },
        }
    }

    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

/// Convert a checked wire value into the config-chain scalar.
fn to_scalar(v: &SettingValue) -> ScalarValue {
    match v {
        SettingValue::Bool(b) => ScalarValue::Bool(*b),
        SettingValue::Int(i) => ScalarValue::Int(*i),
        SettingValue::Float(f) => ScalarValue::Float(*f),
        SettingValue::Str(s) => ScalarValue::Str(s.clone()),
    }
}

#[cfg(test)]
mod tests;
