// SPDX-License-Identifier: AGPL-3.0-only

//! The configuration chain: how a recipe's settings become a resolved config.

use crate::scalar::ScalarValue;
use std::collections::BTreeMap;

/// Values supplied on the command line or by an agent request.
pub type Overrides = BTreeMap<String, ScalarValue>;

/// A user's persistent defaults, applied beneath CLI overrides.
pub type UserConfig = BTreeMap<String, ScalarValue>;

/// The settings a launch will actually use.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedConfig(BTreeMap<String, ScalarValue>);

impl ResolvedConfig {
    /// Resolve the chain: CLI overrides beat user config, which beats the
    /// recipe's own `defaults:`.
    ///
    /// Recipe defaults are explicit data the user chose by naming the recipe,
    /// not an implicit code fallback — which is why this layering is legitimate
    /// under a no-hidden-defaults rule. Nothing here invents a value that
    /// appears in none of the three layers.
    pub fn resolve(
        recipe_defaults: &BTreeMap<String, ScalarValue>,
        user: &UserConfig,
        overrides: &Overrides,
    ) -> Self {
        let mut merged = recipe_defaults.clone();
        for (k, v) in user {
            merged.insert(k.clone(), v.clone());
        }
        for (k, v) in overrides {
            merged.insert(k.clone(), v.clone());
        }
        Self(normalize(merged))
    }

    /// Borrow the resolved map.
    pub fn as_map(&self) -> &BTreeMap<String, ScalarValue> {
        &self.0
    }

    /// Consume into the resolved map.
    pub fn into_map(self) -> BTreeMap<String, ScalarValue> {
        self.0
    }

    /// Look up one resolved value.
    pub fn get(&self, key: &str) -> Option<&ScalarValue> {
        self.0.get(key)
    }
}

/// Fold alias spellings onto their canonical keys.
///
/// `auth_token` is the runtime-native name and `api_key` the portable one; both
/// are accepted, and the canonical key wins when a recipe sets both. Doing this
/// once, here, is what keeps the flag table free of alias special-cases.
fn normalize(mut cfg: BTreeMap<String, ScalarValue>) -> BTreeMap<String, ScalarValue> {
    if let Some(alias) = cfg.remove("auth_token") {
        cfg.entry("api_key".to_string()).or_insert(alias);
    }
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pairs: &[(&str, ScalarValue)]) -> BTreeMap<String, ScalarValue> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn recipe_defaults_apply_when_nothing_overrides_them() {
        let r = ResolvedConfig::resolve(
            &m(&[("port", ScalarValue::Int(8888))]),
            &UserConfig::new(),
            &Overrides::new(),
        );
        assert_eq!(r.get("port"), Some(&ScalarValue::Int(8888)));
    }

    #[test]
    fn cli_overrides_beat_user_config_which_beats_recipe_defaults() {
        let recipe = m(&[("port", ScalarValue::Int(1))]);
        let user = m(&[("port", ScalarValue::Int(2))]);
        let cli = m(&[("port", ScalarValue::Int(3))]);

        assert_eq!(
            ResolvedConfig::resolve(&recipe, &user, &Overrides::new()).get("port"),
            Some(&ScalarValue::Int(2))
        );
        assert_eq!(
            ResolvedConfig::resolve(&recipe, &user, &cli).get("port"),
            Some(&ScalarValue::Int(3))
        );
    }

    #[test]
    fn an_override_can_turn_a_toggle_off() {
        // Overriding to false must be honoured, not treated as "unset".
        let r = ResolvedConfig::resolve(
            &m(&[("speculative", ScalarValue::Bool(true))]),
            &UserConfig::new(),
            &m(&[("speculative", ScalarValue::Bool(false))]),
        );
        assert_eq!(r.get("speculative"), Some(&ScalarValue::Bool(false)));
        assert!(!r.get("speculative").unwrap().is_truthy());
    }

    #[test]
    fn the_auth_token_alias_folds_onto_api_key() {
        let r = ResolvedConfig::resolve(
            &m(&[("auth_token", ScalarValue::Str("sk-x".into()))]),
            &UserConfig::new(),
            &Overrides::new(),
        );
        assert_eq!(r.get("api_key"), Some(&ScalarValue::Str("sk-x".into())));
        assert_eq!(
            r.get("auth_token"),
            None,
            "the alias must not survive resolution"
        );
    }

    #[test]
    fn the_canonical_key_wins_when_both_spellings_are_set() {
        let r = ResolvedConfig::resolve(
            &m(&[
                ("api_key", ScalarValue::Str("canonical".into())),
                ("auth_token", ScalarValue::Str("alias".into())),
            ]),
            &UserConfig::new(),
            &Overrides::new(),
        );
        assert_eq!(
            r.get("api_key"),
            Some(&ScalarValue::Str("canonical".into()))
        );
    }

    #[test]
    fn resolution_invents_nothing() {
        let r = ResolvedConfig::resolve(&BTreeMap::new(), &UserConfig::new(), &Overrides::new());
        assert!(
            r.as_map().is_empty(),
            "an empty chain must resolve to nothing at all"
        );
    }
}
