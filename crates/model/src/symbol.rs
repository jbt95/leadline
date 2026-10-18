//! Stable symbol keys for test impact analysis.
//!
//! A key names one function the way a recorded test association needs it: a
//! body-only edit leaves it unchanged, so a test that covered `charge` still
//! covers `charge` after its body changes. One constructor builds every key,
//! so the shape cannot drift between producers.
//!
//! ```text
//! path:name(ParamType,ParamType)     when the grammar exposes parameter types
//! path:name/arity                    when it does not
//! ```
//!
//! The arity form collides for same-arity overloads wherever a grammar
//! exposes no type spelling (plain JavaScript, unannotated Python, Go grouped
//! names without types), and a function with no parameters has no type to
//! spell, so it keys on arity too. The collision direction is safe: a
//! collapsed key over-selects, it never drops a test that should run.
//!
//! **Anonymous functions are not keyable.** `parse` names them
//! `<anonymous@LINE:COLUMN>`, so their name is position-derived and changes
//! under any edit above them. Such a key must never be recorded in a test
//! association; a changed anonymous function escalates to its enclosing
//! *named* function instead. Lambdas bound to a variable already carry the
//! variable name and are as stable as any other key.

/// The inputs one key is built from.
pub struct SymbolParts<'a> {
    /// Display path of the file holding the function.
    pub path: &'a str,
    /// The name `parse` resolved, including its `<anonymous@LINE:COLUMN>` form.
    pub name: &'a str,
    /// Parameter count, used when no types are exposed.
    pub arity: u32,
    /// One type spelling per parameter, in source order. Empty means the
    /// grammar exposes no types for this function, which falls back to arity.
    pub parameter_types: &'a [String],
}

/// Builds the stable key for one function.
pub fn symbol_key(parts: &SymbolParts<'_>) -> String {
    if parts.parameter_types.is_empty() {
        return format!("{}:{}/{}", parts.path, parts.name, parts.arity);
    }
    format!(
        "{}:{}({})",
        parts.path,
        parts.name,
        parts.parameter_types.join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(path: &str, name: &str, types: &[&str]) -> String {
        let types: Vec<String> = types.iter().map(|text| (*text).to_owned()).collect();
        symbol_key(&SymbolParts {
            path,
            name,
            arity: types.len() as u32,
            parameter_types: &types,
        })
    }

    #[test]
    fn types_are_spelled_when_the_grammar_exposes_them() {
        assert_eq!(
            typed("src/billing.java", "charge", &["int", "String"]),
            "src/billing.java:charge(int,String)"
        );
    }

    #[test]
    fn arity_is_used_when_no_types_are_exposed() {
        assert_eq!(
            symbol_key(&SymbolParts {
                path: "web/app.js",
                name: "charge",
                arity: 2,
                parameter_types: &[],
            }),
            "web/app.js:charge/2"
        );
        // No parameters means no type to spell, so the key keeps the arity
        // form even where the grammar would expose types.
        assert_eq!(
            symbol_key(&SymbolParts {
                path: "src/billing.java",
                name: "charge",
                arity: 0,
                parameter_types: &[],
            }),
            "src/billing.java:charge/0"
        );
    }

    #[test]
    fn parameter_types_distinguish_same_arity_keys() {
        assert_ne!(
            typed("src/billing.java", "charge", &["int"]),
            typed("src/billing.java", "charge", &["String"])
        );
    }

    #[test]
    fn the_path_is_part_of_the_key() {
        assert_ne!(
            typed("src/a.java", "charge", &["int"]),
            typed("src/b.java", "charge", &["int"])
        );
    }
}
