use crate::parser::ast::Package;
use crate::parser::{parse_dir_recursive, Result};
use git2::build::RepoBuilder;
use git2::FetchOptions;
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

#[derive(Debug, Clone)]
struct Require {
    module_path: String,
    version: String,
}

/// Resolves Go import paths to filesystem paths using the module path
/// declared in go.mod. Supports both local and remote dependencies.
#[derive(Debug, Clone)]
pub struct ModuleResolver {
    module_path: String,
    project_root: PathBuf,
    requires: Vec<Require>,
    cache_dir: PathBuf,
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
        let requires = parse_requires(&content);

        let cache_dir = std::env::var("GOPATH")
            .map(|gopath| PathBuf::from(gopath).join("pkg").join("mod").join("cache"))
            .unwrap_or_else(|_| {
                dirs_cache_fallback()
            });

        Some(Self {
            module_path,
            project_root: project_root.to_path_buf(),
            requires,
            cache_dir,
        })
    }

    /// Resolves a Go import path to a filesystem path.
    ///
    /// First tries local resolution (within this module), then
    /// checks require directives for remote dependencies.
    pub fn resolve_import(&self, import_path: &str) -> std::result::Result<PathBuf, String> {
        if let Some(rel) = import_path.strip_prefix(&self.module_path) {
            let rel = rel.trim_start_matches('/');
            let fs_path = if rel.is_empty() {
                self.project_root.clone()
            } else {
                self.project_root.join(rel)
            };
            return Ok(fs_path);
        }

        if let Some(req) = self.find_require(import_path) {
            let cached = self.cached_module_path(&req.module_path, &req.version);

            if !cached.exists() {
                self.download_module(&req.module_path, &req.version)?;
            }

            let subpath = import_path
                .strip_prefix(&req.module_path)
                .unwrap_or("")
                .trim_start_matches('/');

            let fs_path = if subpath.is_empty() {
                cached
            } else {
                cached.join(subpath)
            };

            return Ok(fs_path);
        }

        Err(format!(
            "import '{}' not found: not in module '{}' and no matching require directive",
            import_path, self.module_path
        ))
    }

    pub fn module_path(&self) -> &str {
        &self.module_path
    }

    fn find_require(&self, import_path: &str) -> Option<&Require> {
        self.requires
            .iter()
            .filter(|r| import_path.starts_with(&r.module_path))
            .max_by_key(|r| r.module_path.len())
    }

    fn cached_module_path(&self, module_path: &str, version: &str) -> PathBuf {
        let encoded = module_path.replace('/', "_");
        let dir_name = if version.is_empty() {
            encoded
        } else {
            format!("{}@{}", encoded, version)
        };
        self.cache_dir.join("download").join(dir_name)
    }

    fn download_module(&self, module_path: &str, version: &str) -> std::result::Result<(), String> {
        let url = module_to_git_url(module_path);
        let destination = self.cached_module_path(module_path, version);

        fs::create_dir_all(&destination)
            .map_err(|e| format!("failed to create cache directory: {}", e))?;

        let fetch_options = FetchOptions::new();
        let repo = RepoBuilder::new()
            .fetch_options(fetch_options)
            .clone(&url, &destination)
            .map_err(|e| format!("failed to clone '{}': {}", url, e))?;

        if !version.is_empty() && !version.starts_with("v0.0.0-") {
            let obj = repo
                .revparse_single(version)
                .map_err(|e| format!("failed to find version '{}': {}", version, e))?;
            repo.checkout_tree(&obj, None)
                .map_err(|e| format!("failed to checkout '{}': {}", version, e))?;
            repo.set_head_detached(obj.id())
                .map_err(|e| format!("failed to set HEAD to '{}': {}", version, e))?;
        }

        Ok(())
    }
}

fn dirs_cache_fallback() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join("go").join("pkg").join("mod").join("cache")
    } else {
        PathBuf::from(".govm_cache")
    }
}

fn module_to_git_url(module_path: &str) -> String {
    let parts: Vec<&str> = module_path.splitn(4, '/').collect();
    match parts.first() {
        Some(&"github.com") | Some(&"gitlab.com") | Some(&"bitbucket.org") => {
            if parts.len() >= 3 {
                format!("https://{}/{}/{}", parts[0], parts[1], parts[2])
            } else {
                format!("https://{}", module_path)
            }
        }
        _ => format!("https://{}", module_path),
    }
}

/// Extracts the module path from go.mod content.
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

/// Parses all `require` directives from go.mod content.
/// Handles both single-line and block forms:
///   require github.com/foo/bar v1.2.3
///   require (
///       github.com/foo/bar v1.2.3
///       github.com/baz/qux v0.1.0
///   )
fn parse_requires(content: &str) -> Vec<Require> {
    let mut requires = Vec::new();
    let mut in_block = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("require (") || trimmed == "require (" {
            in_block = true;
            continue;
        }

        if in_block {
            if trimmed == ")" {
                in_block = false;
                continue;
            }
            if trimmed.is_empty() || trimmed.starts_with("//") {
                continue;
            }
            if let Some(req) = parse_require_line(trimmed) {
                requires.push(req);
            }
            continue;
        }

        if trimmed.starts_with("require ") && !trimmed.contains('(') {
            let rest = trimmed.strip_prefix("require ").unwrap().trim();
            if let Some(req) = parse_require_line(rest) {
                requires.push(req);
            }
        }
    }

    requires
}

fn parse_require_line(line: &str) -> Option<Require> {
    let line = line.split("//").next()?.trim();
    let parts: Vec<&str> = line.split_whitespace().collect();
    match parts.len() {
        2 => Some(Require {
            module_path: parts[0].to_string(),
            version: parts[1].to_string(),
        }),
        1 if !parts[0].is_empty() => Some(Require {
            module_path: parts[0].to_string(),
            version: String::new(),
        }),
        _ => None,
    }
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
    fn test_parse_requires_single() {
        let content = "module example.com/foo\nrequire github.com/bar/baz v1.2.3\n";
        let reqs = parse_requires(content);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].module_path, "github.com/bar/baz");
        assert_eq!(reqs[0].version, "v1.2.3");
    }

    #[test]
    fn test_parse_requires_block() {
        let content = "module example.com/foo\n\nrequire (\n\tgithub.com/a/b v1.0.0\n\tgithub.com/c/d v2.0.0\n)\n";
        let reqs = parse_requires(content);
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].module_path, "github.com/a/b");
        assert_eq!(reqs[0].version, "v1.0.0");
        assert_eq!(reqs[1].module_path, "github.com/c/d");
        assert_eq!(reqs[1].version, "v2.0.0");
    }

    #[test]
    fn test_parse_requires_with_comments() {
        let content = "module example.com/foo\nrequire (\n\tgithub.com/a/b v1.0.0 // indirect\n)\n";
        let reqs = parse_requires(content);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].module_path, "github.com/a/b");
        assert_eq!(reqs[0].version, "v1.0.0");
    }

    #[test]
    fn test_module_to_git_url() {
        assert_eq!(
            module_to_git_url("github.com/foo/bar"),
            "https://github.com/foo/bar"
        );
        assert_eq!(
            module_to_git_url("github.com/foo/bar/pkg"),
            "https://github.com/foo/bar"
        );
        assert_eq!(
            module_to_git_url("gitlab.com/org/repo"),
            "https://gitlab.com/org/repo"
        );
    }

    #[test]
    fn test_resolve_import_subpackage() {
        let resolver = ModuleResolver {
            module_path: "github.com/gnolang/gno-rs/local_importing".to_string(),
            project_root: PathBuf::from("/project"),
            requires: vec![],
            cache_dir: PathBuf::from("/tmp/cache"),
        };
        let result = resolver.resolve_import("github.com/gnolang/gno-rs/local_importing/add");
        assert_eq!(result.unwrap(), PathBuf::from("/project/add"));
    }

    #[test]
    fn test_resolve_import_root() {
        let resolver = ModuleResolver {
            module_path: "github.com/gnolang/gno-rs/local_importing".to_string(),
            project_root: PathBuf::from("/project"),
            requires: vec![],
            cache_dir: PathBuf::from("/tmp/cache"),
        };
        let result = resolver.resolve_import("github.com/gnolang/gno-rs/local_importing");
        assert_eq!(result.unwrap(), PathBuf::from("/project"));
    }

    #[test]
    fn test_resolve_import_outside_module() {
        let resolver = ModuleResolver {
            module_path: "github.com/gnolang/gno-rs/local_importing".to_string(),
            project_root: PathBuf::from("/project"),
            requires: vec![],
            cache_dir: PathBuf::from("/tmp/cache"),
        };
        let result = resolver.resolve_import("github.com/other/pkg");
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_require_match() {
        let resolver = ModuleResolver {
            module_path: "example.com/mymod".to_string(),
            project_root: PathBuf::from("/project"),
            requires: vec![Require {
                module_path: "github.com/foo/bar".to_string(),
                version: "v1.0.0".to_string(),
            }],
            cache_dir: PathBuf::from("/tmp/cache"),
        };
        let req = resolver.find_require("github.com/foo/bar/pkg/sub");
        assert!(req.is_some());
        assert_eq!(req.unwrap().module_path, "github.com/foo/bar");
    }

    #[test]
    fn test_cached_module_path() {
        let resolver = ModuleResolver {
            module_path: "example.com/mymod".to_string(),
            project_root: PathBuf::from("/project"),
            requires: vec![],
            cache_dir: PathBuf::from("/tmp/cache"),
        };
        let p = resolver.cached_module_path("github.com/foo/bar", "v1.0.0");
        assert_eq!(p, PathBuf::from("/tmp/cache/download/github.com_foo_bar@v1.0.0"));
    }

    #[test]
    fn test_parse_requires_multiple_blocks() {
        let content = "\
module example.com/foo

require (
\tgithub.com/a/b v1.0.0
)

require (
\tgithub.com/c/d v2.0.0
)
";
        let reqs = parse_requires(content);
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].module_path, "github.com/a/b");
        assert_eq!(reqs[1].module_path, "github.com/c/d");
    }

    #[test]
    fn test_parse_requires_mixed_single_and_block() {
        let content = "\
module example.com/foo

require github.com/single/pkg v0.1.0

require (
\tgithub.com/block/pkg v0.2.0
)
";
        let reqs = parse_requires(content);
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].module_path, "github.com/single/pkg");
        assert_eq!(reqs[0].version, "v0.1.0");
        assert_eq!(reqs[1].module_path, "github.com/block/pkg");
        assert_eq!(reqs[1].version, "v0.2.0");
    }

    #[test]
    fn test_parse_requires_empty_block() {
        let content = "module example.com/foo\n\nrequire (\n)\n";
        let reqs = parse_requires(content);
        assert_eq!(reqs.len(), 0);
    }

    #[test]
    fn test_parse_requires_block_only_comments() {
        let content = "\
module example.com/foo

require (
\t// this is a comment
\t// another comment
)
";
        let reqs = parse_requires(content);
        assert_eq!(reqs.len(), 0);
    }

    #[test]
    fn test_parse_require_line_no_version() {
        let req = parse_require_line("github.com/solo/pkg");
        assert!(req.is_some());
        let req = req.unwrap();
        assert_eq!(req.module_path, "github.com/solo/pkg");
        assert_eq!(req.version, "");
    }

    #[test]
    fn test_parse_require_line_empty() {
        assert!(parse_require_line("").is_none());
    }

    #[test]
    fn test_parse_require_line_whitespace_only() {
        assert!(parse_require_line("   ").is_none());
    }

    #[test]
    fn test_parse_module_path_trailing_whitespace() {
        let content = "module   example.com/foo   \n\ngo 1.20\n";
        assert_eq!(
            parse_module_path(content),
            Some("example.com/foo".to_string())
        );
    }

    #[test]
    fn test_find_require_longest_prefix() {
        let resolver = ModuleResolver {
            module_path: "example.com/mymod".to_string(),
            project_root: PathBuf::from("/project"),
            requires: vec![
                Require {
                    module_path: "github.com/foo".to_string(),
                    version: "v1.0.0".to_string(),
                },
                Require {
                    module_path: "github.com/foo/bar".to_string(),
                    version: "v2.0.0".to_string(),
                },
            ],
            cache_dir: PathBuf::from("/tmp/cache"),
        };
        let req = resolver.find_require("github.com/foo/bar/baz");
        assert!(req.is_some());
        assert_eq!(req.unwrap().module_path, "github.com/foo/bar");
        assert_eq!(req.unwrap().version, "v2.0.0");
    }

    #[test]
    fn test_find_require_no_match() {
        let resolver = ModuleResolver {
            module_path: "example.com/mymod".to_string(),
            project_root: PathBuf::from("/project"),
            requires: vec![Require {
                module_path: "github.com/foo/bar".to_string(),
                version: "v1.0.0".to_string(),
            }],
            cache_dir: PathBuf::from("/tmp/cache"),
        };
        let req = resolver.find_require("github.com/other/thing");
        assert!(req.is_none());
    }

    #[test]
    fn test_resolve_import_no_require_returns_error() {
        let resolver = ModuleResolver {
            module_path: "example.com/mymod".to_string(),
            project_root: PathBuf::from("/project"),
            requires: vec![],
            cache_dir: PathBuf::from("/tmp/cache"),
        };
        let result = resolver.resolve_import("github.com/unknown/pkg");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("not found"), "error should mention not found: {}", err);
    }

    #[test]
    fn test_module_to_git_url_non_standard_host() {
        assert_eq!(
            module_to_git_url("golang.org/x/text"),
            "https://golang.org/x/text"
        );
        assert_eq!(
            module_to_git_url("example.com/custom/repo"),
            "https://example.com/custom/repo"
        );
    }

    #[test]
    fn test_module_to_git_url_bitbucket() {
        assert_eq!(
            module_to_git_url("bitbucket.org/user/repo"),
            "https://bitbucket.org/user/repo"
        );
        assert_eq!(
            module_to_git_url("bitbucket.org/user/repo/sub/pkg"),
            "https://bitbucket.org/user/repo"
        );
    }

    #[test]
    fn test_cached_module_path_empty_version() {
        let resolver = ModuleResolver {
            module_path: "example.com/mymod".to_string(),
            project_root: PathBuf::from("/project"),
            requires: vec![],
            cache_dir: PathBuf::from("/tmp/cache"),
        };
        let p = resolver.cached_module_path("github.com/foo/bar", "");
        assert_eq!(p, PathBuf::from("/tmp/cache/download/github.com_foo_bar"));
    }

    #[test]
    fn test_from_project_root_with_temp_dir() {
        let dir = std::env::temp_dir().join("govm_test_resolver");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("go.mod"),
            "module example.com/testmod\n\nrequire github.com/dep/a v1.0.0\n",
        )
        .unwrap();

        let resolver = ModuleResolver::from_project_root(&dir);
        assert!(resolver.is_some());
        let resolver = resolver.unwrap();
        assert_eq!(resolver.module_path(), "example.com/testmod");
        assert_eq!(resolver.requires.len(), 1);
        assert_eq!(resolver.requires[0].module_path, "github.com/dep/a");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_from_project_root_no_gomod() {
        let dir = std::env::temp_dir().join("govm_test_no_gomod");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let resolver = ModuleResolver::from_project_root(&dir);
        assert!(resolver.is_none());

        let _ = fs::remove_dir_all(&dir);
    }
}
