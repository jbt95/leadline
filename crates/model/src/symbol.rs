//! Stable symbol keys for test impact analysis.
//!
//! A key names one function the way a recorded test association needs it: a
//! body-only edit leaves it unchanged, so a test that covered `charge` still
//! covers `charge` after its body changes. One constructor builds every key,
//! so the shape cannot drift between producers.
//!
//! ```text
//! path:Qualifier.name(ParamType,ParamType)   types, inside a named scope
//! path:Qualifier.name/arity                  no types, inside a named scope
//! path:name(ParamType,ParamType)             types, top level
//! path:name/arity                            no types, top level
//! ```
//!
//! `Qualifier` is the nearest enclosing **named** scope chain, outermost
//! first, joined with `.` — a class, `impl`, namespace, the object a member is
//! bound to, or the function a function is nested in. It is omitted entirely
//! for a top-level function, so that key keeps the unqualified form. Every
//! part is a name, never a position or index, which is what keeps a body-only
//! edit from moving the key.
//!
//! Collisions that remain, all of which over-select rather than drop a test:
//!
//! - The arity form collides for same-arity overloads wherever a grammar
//!   exposes no type spelling (plain JavaScript, unannotated Python, Go
//!   grouped names without types), and a function with no parameters has no
//!   type to spell, so it keys on arity too.
//! - Rust keys an `impl Trait for Type` method under `Type` alone, so two
//!   traits implemented for one type with a same-named, same-signature method
//!   share a key.
//! - A C++ definition written outside its class (`void Foo::save() {}`) keys
//!   under the namespace enclosing the definition, not under `Foo`.
//! - An anonymous enclosing function contributes no scope, so two same-named
//!   functions nested in two different anonymous functions share a key.
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
    /// Names of the enclosing scopes, outermost first, or `None` at top level.
    pub qualifier: Option<&'a str>,
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
    let name = match parts.qualifier {
        Some(qualifier) => format!("{qualifier}.{}", parts.name),
        None => parts.name.to_owned(),
    };
    if parts.parameter_types.is_empty() {
        return format!("{}:{}/{}", parts.path, name, parts.arity);
    }
    format!(
        "{}:{}({})",
        parts.path,
        name,
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
            qualifier: None,
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
    fn an_enclosing_scope_qualifies_the_name() {
        let types = ["int".to_owned()];
        assert_eq!(
            symbol_key(&SymbolParts {
                path: "src/billing.java",
                qualifier: Some("Billing"),
                name: "charge",
                arity: 1,
                parameter_types: &types,
            }),
            "src/billing.java:Billing.charge(int)"
        );
        assert_eq!(
            symbol_key(&SymbolParts {
                path: "src/billing.java",
                qualifier: Some("Outer.Inner"),
                name: "charge",
                arity: 0,
                parameter_types: &[],
            }),
            "src/billing.java:Outer.Inner.charge/0"
        );
    }

    #[test]
    fn arity_is_used_when_no_types_are_exposed() {
        assert_eq!(
            symbol_key(&SymbolParts {
                path: "web/app.js",
                qualifier: None,
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
                qualifier: None,
                name: "charge",
                arity: 0,
                parameter_types: &[],
            }),
            "src/billing.java:charge/0"
        );
    }

    #[test]
    fn a_qualifier_separates_same_named_methods_on_different_receivers() {
        assert_ne!(
            symbol_key(&SymbolParts {
                path: "store.go",
                qualifier: Some("User"),
                name: "Save",
                arity: 0,
                parameter_types: &[],
            }),
            symbol_key(&SymbolParts {
                path: "store.go",
                qualifier: Some("Order"),
                name: "Save",
                arity: 0,
                parameter_types: &[],
            })
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
