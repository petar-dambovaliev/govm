use crate::parser::ast::Package;
use crate::parser::{parse_dir_recursive, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub fn parse_local_dependencies(f: &PathBuf) -> Result<Vec<Package>> {
    let main_dir = if f.is_dir() {
        f.as_path()
    } else if f.is_file() {
        f.parent().unwrap()
    } else {
        panic!("invalid path")
    };

    let local_pkgs: Vec<Package> = parse_dir_recursive(main_dir)?
        .iter()
        .map(|p| p.1.clone())
        .collect();

    Ok(local_pkgs)
}

/// Resolves Go import paths to filesystem paths using the module path
/// declared in go.mod.
#[derive(Debug, Clone)]
pub struct ModuleResolver {
    module_path: String,
    project_root: PathBuf,
}

impl ModuleResolver {
    /// Reads go.mod from `project_root` and creates a resolver.
    /// Returns `None` if go.mod does not exist.
    pub fn from_project_root(project_root: &Path) -> Option<Self> {
        let go_mod = project_root.join("go.mod");
        if !go_mod.exists() {
            return None;
        }

        let content = fs::read_to_string(&go_mod).ok()?;
        let module_path = parse_module_path(&content)?;

        Some(Self {
            module_path,
            project_root: project_root.to_path_buf(),
        })
    }

    /// Resolves a Go import path to a filesystem path.
    ///
    /// For an import like `"github.com/foo/mymod/pkg/sub"` and a module path
    /// `"github.com/foo/mymod"`, this strips the prefix to get `"pkg/sub"` and
    /// joins it to the project root.
    pub fn resolve_import(&self, import_path: &str) -> std::result::Result<PathBuf, String> {
        if let Some(rel) = import_path.strip_prefix(&self.module_path) {
            let rel = rel.trim_start_matches('/');
            let fs_path = if rel.is_empty() {
                self.project_root.clone()
            } else {
                self.project_root.join(rel)
            };
            Ok(fs_path)
        } else {
            Err(format!(
                "import '{}' is outside module '{}'",
                import_path, self.module_path
            ))
        }
    }

    pub fn module_path(&self) -> &str {
        &self.module_path
    }
}

/// Extracts the module path from go.mod content.
/// Looks for a line like `module github.com/foo/bar`.
fn parse_module_path(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("module ") {
            let module = trimmed["module ".len()..].trim();
            if !module.is_empty() {
                return Some(module.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_module_path() {
        let content = "module github.com/gnolang/gno-rs/local_importing\n\ngo 1.20\n";
        assert_eq!(
            parse_module_path(content),
            Some("github.com/gnolang/gno-rs/local_importing".to_string())
        );
    }

    #[test]
    fn test_parse_module_path_no_module() {
        assert_eq!(parse_module_path("go 1.20\n"), None);
    }

    #[test]
    fn test_resolve_import_subpackage() {
        let resolver = ModuleResolver {
            module_path: "github.com/gnolang/gno-rs/local_importing".to_string(),
            project_root: PathBuf::from("/project"),
        };
        let result = resolver.resolve_import("github.com/gnolang/gno-rs/local_importing/add");
        assert_eq!(result.unwrap(), PathBuf::from("/project/add"));
    }

    #[test]
    fn test_resolve_import_root() {
        let resolver = ModuleResolver {
            module_path: "github.com/gnolang/gno-rs/local_importing".to_string(),
            project_root: PathBuf::from("/project"),
        };
        let result = resolver.resolve_import("github.com/gnolang/gno-rs/local_importing");
        assert_eq!(result.unwrap(), PathBuf::from("/project"));
    }

    #[test]
    fn test_resolve_import_outside_module() {
        let resolver = ModuleResolver {
            module_path: "github.com/gnolang/gno-rs/local_importing".to_string(),
            project_root: PathBuf::from("/project"),
        };
        let result = resolver.resolve_import("github.com/other/pkg");
        assert!(result.is_err());
    }
}
