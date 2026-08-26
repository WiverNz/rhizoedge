//! The file and environment layers, as `figment` providers.
//!
//! Written out rather than using `figment::providers::Env` directly for one
//! reason: `Env` reads the real process environment, which would make every
//! layering test depend on the developer's shell and require mutating the
//! environment from a test. Taking the variables as an argument keeps the
//! precedence tests hermetic and parallel-safe.
//!
//! Both providers carry [`Metadata`] naming their source, so a malformed value
//! produces an error that says *which* layer supplied it — the thing that
//! decided `figment` over `config` in the first place.

use figment::value::{Dict, Map, Value};
use figment::{Error, Metadata, Profile, Provider};

/// A pre-built dictionary presented as a named layer.
pub(super) struct NamedDict {
    name: String,
    dict: Dict,
}

impl NamedDict {
    pub(super) fn new(name: impl Into<String>, dict: Dict) -> Self {
        Self {
            name: name.into(),
            dict,
        }
    }
}

impl Provider for NamedDict {
    fn metadata(&self) -> Metadata {
        Metadata::named(self.name.clone())
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        Ok(Profile::Default.collect(self.dict.clone()))
    }
}

/// Environment variables, filtered by prefix and nested on a separator.
///
/// `RHIZO_EDGE__MQTT__BROKER_URL=mqtt://host:1883` becomes
/// `mqtt.broker_url = "mqtt://host:1883"`.
///
/// Values are parsed with `figment`'s own `Value` grammar, so `true` becomes a
/// boolean and `30` a number — exactly as `figment::providers::Env` does, and
/// for the same reason: a typed field must be settable from a variable.
pub(super) struct PrefixedEnv {
    prefix: String,
    separator: String,
    vars: Vec<(String, String)>,
}

impl PrefixedEnv {
    pub(super) fn new<I>(prefix: &str, separator: &str, vars: I) -> Self
    where
        I: IntoIterator<Item = (String, String)>,
    {
        Self {
            prefix: prefix.to_owned(),
            separator: separator.to_owned(),
            vars: vars.into_iter().collect(),
        }
    }

    /// The dotted key paths this layer will set, in the order encountered.
    ///
    /// Lets the nesting rule be asserted directly, rather than only through
    /// its effect on a fully-extracted `EdgeConfig`.
    #[cfg(test)]
    fn keys(&self) -> Vec<String> {
        self.vars
            .iter()
            .filter_map(|(k, _)| self.key_path(k))
            .map(|parts| parts.join("."))
            .collect()
    }

    /// Splits a variable name into its dotted path, or `None` if the prefix
    /// does not match.
    fn key_path(&self, var: &str) -> Option<Vec<String>> {
        let rest = var.strip_prefix(&self.prefix)?;
        if rest.is_empty() {
            return None;
        }
        Some(
            rest.split(&self.separator)
                .map(|p| p.to_ascii_lowercase())
                .collect(),
        )
    }
}

impl Provider for PrefixedEnv {
    fn metadata(&self) -> Metadata {
        Metadata::named(format!("`{}` environment variable(s)", self.prefix))
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut root = Dict::new();
        for (var, raw) in &self.vars {
            let Some(path) = self.key_path(var) else {
                continue;
            };
            // `Value: FromStr` is infallible and is what gives `true` a
            // boolean and `30` a number.
            let value: Value = raw.parse().unwrap_or_else(|_| Value::from(raw.as_str()));
            insert_nested(&mut root, &path, value);
        }
        Ok(Profile::Default.collect(root))
    }
}

/// Inserts `value` at `path`, creating intermediate dictionaries.
///
/// A scalar already sitting where a dictionary is needed is replaced: two
/// variables that disagree about whether a key is a leaf is a configuration
/// error the extraction step will report against the resulting type, which is
/// a better message than anything this function could produce.
fn insert_nested(root: &mut Dict, path: &[String], value: Value) {
    let Some((leaf, parents)) = path.split_last() else {
        return;
    };
    let mut cursor = root;
    for part in parents {
        let entry = cursor
            .entry(part.clone())
            .or_insert_with(|| Value::from(Dict::new()));
        if !matches!(entry, Value::Dict(..)) {
            *entry = Value::from(Dict::new());
        }
        let Value::Dict(_, inner) = entry else {
            return;
        };
        cursor = inner;
    }
    cursor.insert(leaf.clone(), value);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn a_double_underscore_becomes_a_nesting_level() {
        let env = PrefixedEnv::new(
            "RHIZO_EDGE__",
            "__",
            vars(&[("RHIZO_EDGE__MQTT__BROKER_URL", "mqtt://host:1883")]),
        );
        assert_eq!(env.keys(), vec!["mqtt.broker_url"]);
    }

    #[test]
    fn variables_without_the_prefix_are_ignored() {
        let env = PrefixedEnv::new(
            "RHIZO_EDGE__",
            "__",
            vars(&[
                ("PATH", "/usr/bin"),
                ("RHIZO_SIM__DEVICE_ID", "plant-node-01"),
                ("RHIZO_EDGE__EDGE_ID", "home-02"),
            ]),
        );
        assert_eq!(env.keys(), vec!["edge_id"]);
    }

    #[test]
    fn values_are_typed_not_left_as_strings() {
        let env = PrefixedEnv::new(
            "RHIZO_EDGE__",
            "__",
            vars(&[
                ("RHIZO_EDGE__CLOUD__ENABLED", "true"),
                ("RHIZO_EDGE__CONTROL__TICK_INTERVAL_SECONDS", "45"),
                ("RHIZO_EDGE__LOG__LEVEL", "debug"),
            ]),
        );
        let data = env.data().unwrap();
        let dict = &data[&Profile::Default];
        let Value::Dict(_, cloud) = &dict["cloud"] else {
            panic!("cloud must be a table");
        };
        assert!(matches!(cloud["enabled"], Value::Bool(_, true)));

        let Value::Dict(_, control) = &dict["control"] else {
            panic!("control must be a table");
        };
        assert!(matches!(control["tick_interval_seconds"], Value::Num(..)));

        let Value::Dict(_, log) = &dict["log"] else {
            panic!("log must be a table");
        };
        assert!(matches!(log["level"], Value::String(_, _)));
    }

    #[test]
    fn several_variables_merge_into_one_table() {
        let env = PrefixedEnv::new(
            "RHIZO_EDGE__",
            "__",
            vars(&[
                ("RHIZO_EDGE__MQTT__BROKER_URL", "mqtt://a:1883"),
                ("RHIZO_EDGE__MQTT__USERNAME", "rhizo-edge"),
            ]),
        );
        let data = env.data().unwrap();
        let Value::Dict(_, mqtt) = &data[&Profile::Default]["mqtt"] else {
            panic!("mqtt must be a table");
        };
        assert_eq!(mqtt.len(), 2);
    }

    #[test]
    fn the_layer_names_itself_in_metadata() {
        let env = PrefixedEnv::new("RHIZO_EDGE__", "__", vars(&[]));
        assert!(env.metadata().name.contains("RHIZO_EDGE__"));
        assert!(env.metadata().name.contains("environment"));
    }
}
