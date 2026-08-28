// SPDX-License-Identifier: AGPL-3.0-only

//! Pure argument validation, separated from the commands that use it.

use anyhow::{Result, bail};
use atlasctl_core::ScalarValue;
use atlasctl_core::chain::Overrides;
use atlasctl_core::docker::translate::Placement;

/// Parse `KEY=VALUE` into a typed override.
///
/// The value is parsed as a YAML scalar so `-o speculative=true` is a boolean
/// and `-o gpu_memory_utilization=0.85` a float. Typing them here rather than
/// treating everything as a string is what lets a bare toggle be switched off
/// from the command line at all.
pub fn parse_override(input: &str) -> Result<(String, ScalarValue)> {
    let Some((key, raw)) = input.split_once('=') else {
        bail!("expected KEY=VALUE, got `{input}`");
    };
    let key = key.trim();
    if key.is_empty() {
        bail!("empty key in `{input}`");
    }
    let value = serde_yaml_ng::from_str::<ScalarValue>(raw)
        .unwrap_or_else(|_| ScalarValue::Str(raw.to_string()));
    Ok((key.to_string(), value))
}

/// Merge `-o` options with the typed convenience flags.
///
/// A key set both ways is an error rather than a silent precedence rule: if
/// someone writes `--port 8000 -o port=9000` they have contradicted themselves,
/// and picking a winner would hide that.
pub fn build_overrides(options: &[String], port: Option<u16>) -> Result<Overrides> {
    let mut out = Overrides::new();
    for opt in options {
        let (k, v) = parse_override(opt)?;
        // The declared bound was enforced for the web surface and not here, so
        // the CLI passed a nonsense value straight into the rendered command.
        atlasctl_core::settings::check_override(&k, &v)?;
        if out.insert(k.clone(), v).is_some() {
            bail!("`{k}` was given more than once");
        }
    }
    if let Some(p) = port {
        if out.contains_key("port") {
            bail!("port was given both as `--port` and as `-o port=...`; use one");
        }
        out.insert("port".to_string(), ScalarValue::Int(i64::from(p)));
    }
    Ok(out)
}

/// Build the placement from the rank flags.
pub fn build_placement(
    rank: Option<u16>,
    world_size: Option<u16>,
    master_addr: Option<String>,
    master_port: u16,
) -> Result<Placement> {
    match (rank, world_size, master_addr) {
        (None, None, None) => Ok(Placement::Solo),
        (Some(rank), Some(world_size), Some(master_addr)) => {
            if world_size < 2 {
                bail!("--world-size must be at least 2 for a multi-node launch");
            }
            if rank >= world_size {
                bail!("--rank {rank} is out of range for --world-size {world_size}");
            }
            Ok(Placement::Rank {
                rank,
                world_size,
                master_addr,
                master_port,
            })
        }
        _ => bail!("--rank, --world-size and --master-addr must be given together"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_keep_their_yaml_types() {
        assert_eq!(
            parse_override("port=8888").unwrap().1,
            ScalarValue::Int(8888)
        );
        assert_eq!(
            parse_override("g=0.85").unwrap().1,
            ScalarValue::Float(0.85)
        );
        assert_eq!(
            parse_override("speculative=true").unwrap().1,
            ScalarValue::Bool(true)
        );
        assert_eq!(
            parse_override("kv_cache_dtype=fp8").unwrap().1,
            ScalarValue::Str("fp8".into())
        );
    }

    #[test]
    fn a_value_containing_equals_survives_intact() {
        assert_eq!(
            parse_override("k=a=b").unwrap(),
            ("k".to_string(), ScalarValue::Str("a=b".into()))
        );
    }

    #[test]
    fn malformed_overrides_are_rejected() {
        assert!(parse_override("noequals").is_err());
        assert!(parse_override("=novalue").is_err());
    }

    #[test]
    fn a_toggle_can_be_switched_off_from_the_command_line() {
        let o = build_overrides(&["speculative=false".into()], None).unwrap();
        assert_eq!(o["speculative"], ScalarValue::Bool(false));
    }

    #[test]
    fn contradicting_yourself_about_the_port_is_an_error() {
        let err = build_overrides(&["port=9000".into()], Some(8000)).expect_err("must fail");
        assert!(err.to_string().contains("use one"), "{err}");
    }

    #[test]
    fn repeating_the_same_option_is_an_error() {
        let err = build_overrides(&["a=1".into(), "a=2".into()], None).expect_err("must fail");
        assert!(err.to_string().contains("more than once"), "{err}");
    }

    #[test]
    fn no_rank_flags_means_a_solo_launch() {
        assert_eq!(
            build_placement(None, None, None, 29500).unwrap(),
            Placement::Solo
        );
    }

    #[test]
    fn rank_flags_must_be_complete_and_consistent() {
        assert!(build_placement(Some(0), None, None, 29500).is_err());
        assert!(build_placement(Some(2), Some(2), Some("h".into()), 29500).is_err());
        assert!(build_placement(Some(0), Some(1), Some("h".into()), 29500).is_err());
        assert!(build_placement(Some(1), Some(2), Some("h".into()), 29500).is_ok());
    }
}
