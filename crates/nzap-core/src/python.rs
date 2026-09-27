//! Safe embedding of user values into generated Python.
//!
//! Every user-supplied value that ends up in a cell the engine generates
//! (package names, mount paths, env vars, argv, notebook parameters) goes
//! through [`literal`], so it can only ever be a Python string literal.

/// A Python string literal for `value`.
///
/// JSON string syntax is a subset of Python's: `"`, `\\`, `\n`, `\r`, `\t`,
/// `\b`, `\f` and `\uXXXX` escapes mean the same thing in both, and
/// serde_json never emits the one JSON escape Python lacks (`\/`).
pub fn literal(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

/// A Python list of string literals: `["a", "b"]`.
pub fn list(values: &[String]) -> String {
    let items: Vec<String> = values.iter().map(|value| literal(value)).collect();
    format!("[{}]", items.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_cannot_break_out() {
        assert_eq!(literal("plain"), "\"plain\"");
        assert_eq!(literal("a\"b"), "\"a\\\"b\"");
        assert_eq!(literal("x\n__import__('os')"), "\"x\\n__import__('os')\"");
        assert_eq!(literal("back\\slash"), "\"back\\\\slash\"");
        assert_eq!(literal("\u{0}"), "\"\\u0000\"");
        assert_eq!(literal("/content/drive"), "\"/content/drive\"");
        assert_eq!(list(&["a".into(), "b c".into()]), "[\"a\", \"b c\"]");
        assert_eq!(list(&[]), "[]");
    }
}
