use std::collections::HashSet;

lazy_static::lazy_static! {
    static ref ALLOWED_STDLIB: HashSet<&'static str> = {
        let mut s = HashSet::new();
        s.insert("strings");
        s.insert("strconv");
        s.insert("math");
        s.insert("sort");
        s.insert("fmt");
        s.insert("unicode");
        s.insert("unicode/utf8");
        s.insert("bytes");
        s.insert("encoding/json");
        s.insert("errors");
        s.insert("time");
        s
    };

    static ref REJECTED_WITH_REASON: Vec<(&'static str, &'static str)> = vec![
        ("os", "I/O not allowed in sandboxed UDFs"),
        ("os/exec", "process execution not allowed in sandboxed UDFs"),
        ("os/user", "user lookup not allowed in sandboxed UDFs"),
        ("io", "I/O not allowed in sandboxed UDFs"),
        ("net", "network access not allowed in sandboxed UDFs"),
        ("net/http", "network access not allowed in sandboxed UDFs"),
        ("syscall", "system calls not allowed in sandboxed UDFs"),
        ("runtime", "runtime access not allowed in sandboxed UDFs"),
        ("unsafe", "unsafe operations not allowed in sandboxed UDFs"),
        ("reflect", "reflection not supported in WASM UDFs"),
        ("sync", "concurrency not available in WASM UDFs"),
        ("sync/atomic", "atomic operations not available in WASM UDFs"),
        ("crypto", "crypto not allowed in sandboxed UDFs"),
        ("plugin", "plugins not supported in WASM UDFs"),
        ("database/sql", "database access not allowed in sandboxed UDFs"),
    ];
}

const UDF_IMPORT_PREFIX: &str = "udf/";

include!("stdlib_generated.rs");

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportKind {
    Stdlib(String),
    Udf(String),
}

pub fn resolve_import(path: &str) -> Result<ImportKind, String> {
    if path.starts_with(UDF_IMPORT_PREFIX) {
        let udf_name = &path[UDF_IMPORT_PREFIX.len()..];
        if udf_name.is_empty() {
            return Err("empty UDF import path".to_string());
        }
        return Ok(ImportKind::Udf(udf_name.to_string()));
    }

    if ALLOWED_STDLIB.contains(path) {
        return Ok(ImportKind::Stdlib(path.to_string()));
    }

    for (pkg, reason) in REJECTED_WITH_REASON.iter() {
        if path == *pkg || path.starts_with(&format!("{}/", pkg)) {
            return Err(format!("import \"{}\" is not allowed: {}", path, reason));
        }
    }

    Err(format!(
        "import \"{}\" is not allowed in UDFs. Only stdlib packages and udf/ imports are permitted.",
        path
    ))
}
