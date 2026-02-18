use crate::parser::ast::Package;
use crate::parser::{parse_dir_recursive, Result};
use git2::build::RepoBuilder;
use git2::FetchOptions;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
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

#[derive(Debug, Clone)]
struct Replace {
    old_path: String,
    old_version: Option<String>,
    new_path: String,
    new_version: Option<String>,
}

#[derive(Debug, Clone)]
struct Exclude {
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
    replaces: Vec<Replace>,
    excludes: Vec<Exclude>,
    cache_dir: PathBuf,
    go_sum_path: PathBuf,
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
        let replaces = parse_replaces(&content);
        let excludes = parse_excludes(&content);

        let cache_dir = std::env::var("GOPATH")
            .map(|gopath| PathBuf::from(gopath).join("pkg").join("mod").join("cache"))
            .unwrap_or_else(|_| {
                dirs_cache_fallback()
            });

        let go_sum_path = project_root.join("go.sum");

        Some(Self {
            module_path,
            project_root: project_root.to_path_buf(),
            requires,
            replaces,
            excludes,
            cache_dir,
            go_sum_path,
        })
    }

    /// Resolves a Go import path to a filesystem path.
    ///
    /// First tries local resolution (within this module), then
    /// checks require directives for remote dependencies.
    pub fn resolve_import(&self, import_path: &str) -> std::result::Result<PathBuf, String> {
        // Check replace directives first
        if let Some(repl) = self.find_replace(import_path) {
            let subpath = import_path
                .strip_prefix(&repl.old_path)
                .unwrap_or("")
                .trim_start_matches('/');

            let is_local = repl.new_path.starts_with("./")
                || repl.new_path.starts_with("../")
                || repl.new_path.starts_with('/');

            if is_local {
                let base = self.project_root.join(&repl.new_path);
                let fs_path = if subpath.is_empty() {
                    base
                } else {
                    base.join(subpath)
                };
                return Ok(fs_path);
            }

            // Remote replacement: treat as a require with the new path/version
            let version = repl.new_version.clone().unwrap_or_default();
            let cached = self.cached_module_path(&repl.new_path, &version);
            if !cached.exists() {
                self.download_module(&repl.new_path, &version)?;
            }
            let fs_path = if subpath.is_empty() {
                cached
            } else {
                cached.join(subpath)
            };
            return Ok(fs_path);
        }

        // Local module resolution
        if let Some(rel) = import_path.strip_prefix(&self.module_path) {
            let rel = rel.trim_start_matches('/');
            let fs_path = if rel.is_empty() {
                self.project_root.clone()
            } else {
                self.project_root.join(rel)
            };
            return Ok(fs_path);
        }

        // Remote require resolution
        if let Some(req) = self.find_require(import_path) {
            if self.is_excluded(&req.module_path, &req.version) {
                return Err(format!(
                    "import '{}': version {} is excluded",
                    import_path, req.version
                ));
            }

            let cached = self.cached_module_path(&req.module_path, &req.version);

            if !cached.exists() {
                self.download_module(&req.module_path, &req.version)?;
            } else {
                self.verify_module(&req.module_path, &req.version)?;
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

        // Try transitive resolution: check if any cached dependency has
        // a go.mod that can resolve this import
        for req in &self.requires {
            let cached = self.cached_module_path(&req.module_path, &req.version);
            if cached.exists() {
                if let Some(sub) = self.sub_resolver(&cached) {
                    if let Ok(path) = sub.resolve_import(import_path) {
                        return Ok(path);
                    }
                }
            }
        }

        Err(format!(
            "import '{}' not found: not in module '{}' and no matching require directive",
            import_path, self.module_path
        ))
    }

    pub fn module_path(&self) -> &str {
        &self.module_path
    }

    /// Creates a sub-resolver for a dependency's go.mod.
    /// Used for transitive dependency resolution.
    pub fn sub_resolver(&self, dep_root: &Path) -> Option<Self> {
        let mut sub = Self::from_project_root(dep_root)?;
        sub.cache_dir = self.cache_dir.clone();
        sub.go_sum_path = self.go_sum_path.clone();
        Some(sub)
    }

    fn find_require(&self, import_path: &str) -> Option<&Require> {
        self.requires
            .iter()
            .filter(|r| import_path.starts_with(&r.module_path))
            .max_by_key(|r| r.module_path.len())
    }

    fn find_replace(&self, import_path: &str) -> Option<&Replace> {
        self.replaces
            .iter()
            .filter(|r| import_path.starts_with(&r.old_path))
            .max_by_key(|r| r.old_path.len())
    }

    fn is_excluded(&self, module_path: &str, version: &str) -> bool {
        self.excludes
            .iter()
            .any(|e| e.module_path == module_path && e.version == version)
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

        // Record hash in go.sum
        if let Ok(hash) = hash_dir(&destination) {
            let key = format!("{} {}", module_path, if version.is_empty() { "v0.0.0" } else { version });
            let mut entries = read_go_sum(&self.go_sum_path);
            entries.insert(key, hash);
            let _ = write_go_sum(&self.go_sum_path, &entries);
        }

        Ok(())
    }

    /// Verifies a cached module's hash against go.sum.
    /// Returns Ok(()) if no go.sum entry exists (first download) or if the hash matches.
    fn verify_module(&self, module_path: &str, version: &str) -> std::result::Result<(), String> {
        let entries = read_go_sum(&self.go_sum_path);
        let key = format!("{} {}", module_path, if version.is_empty() { "v0.0.0" } else { version });

        if let Some(expected_hash) = entries.get(&key) {
            let cached = self.cached_module_path(module_path, version);
            if cached.exists() {
                let actual_hash = hash_dir(&cached)?;
                if &actual_hash != expected_hash {
                    return Err(format!(
                        "checksum mismatch for {} {}: expected {}, got {}",
                        module_path, version, expected_hash, actual_hash
                    ));
                }
            }
        }

        Ok(())
    }
}

/// Computes a SHA-256 hash of a directory's contents (sorted by path).
/// Similar to Go's `dirhash.HashDir`.
fn hash_dir(dir: &Path) -> std::result::Result<String, String> {
    let mut files = BTreeMap::new();
    collect_files(dir, dir, &mut files)
        .map_err(|e| format!("failed to hash directory {:?}: {}", dir, e))?;

    let mut hasher = Sha256::new();
    for (rel_path, contents) in &files {
        hasher.update(format!("file {} {}\n", rel_path, contents.len()));
        hasher.update(contents);
    }

    let hash = hasher.finalize();
    Ok(format!("h1:{}", base64_encode(&hash)))
}

fn collect_files(
    base: &Path,
    dir: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().unwrap().to_str().unwrap_or("");
            if name == ".git" || name == ".hg" || name == ".svn" {
                continue;
            }
            collect_files(base, &path, files)?;
        } else {
            let rel = path.strip_prefix(base).unwrap();
            let rel_str = rel.to_str().unwrap_or("").replace('\\', "/");
            let mut contents = Vec::new();
            let mut f = fs::File::open(&path)?;
            f.read_to_end(&mut contents)?;
            files.insert(rel_str, contents);
        }
    }
    Ok(())
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i] as u32;
        let b1 = if i + 1 < data.len() { data[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < data.len() { data[i + 2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if i + 1 < data.len() {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if i + 2 < data.len() {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        i += 3;
    }
    result
}

/// Reads go.sum entries from a file.
fn read_go_sum(path: &Path) -> BTreeMap<String, String> {
    let mut entries = BTreeMap::new();
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() == 3 {
                let key = format!("{} {}", parts[0], parts[1]);
                entries.insert(key, parts[2].to_string());
            }
        }
    }
    entries
}

/// Writes go.sum entries to a file.
fn write_go_sum(
    path: &Path,
    entries: &BTreeMap<String, String>,
) -> std::result::Result<(), String> {
    let mut content = String::new();
    for (key, hash) in entries {
        content.push_str(&format!("{} {}\n", key, hash));
    }
    fs::write(path, content).map_err(|e| format!("failed to write go.sum: {}", e))
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

/// Parses all `replace` directives from go.mod content.
/// Handles both single-line and block forms:
///   replace github.com/foo/bar => ../local/bar
///   replace github.com/foo/bar v1.0.0 => github.com/fork/bar v1.0.1
///   replace (
///       github.com/foo/bar => ../local/bar
///   )
fn parse_replaces(content: &str) -> Vec<Replace> {
    let mut replaces = Vec::new();
    let mut in_block = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("replace (") || trimmed == "replace (" {
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
            if let Some(r) = parse_replace_line(trimmed) {
                replaces.push(r);
            }
            continue;
        }

        if trimmed.starts_with("replace ") && !trimmed.contains('(') {
            let rest = trimmed.strip_prefix("replace ").unwrap().trim();
            if let Some(r) = parse_replace_line(rest) {
                replaces.push(r);
            }
        }
    }

    replaces
}

fn parse_replace_line(line: &str) -> Option<Replace> {
    let line = line.split("//").next()?.trim();
    let parts: Vec<&str> = line.splitn(2, "=>").collect();
    if parts.len() != 2 {
        return None;
    }

    let left: Vec<&str> = parts[0].trim().split_whitespace().collect();
    let right: Vec<&str> = parts[1].trim().split_whitespace().collect();

    if left.is_empty() || right.is_empty() {
        return None;
    }

    let old_path = left[0].to_string();
    let old_version = if left.len() >= 2 {
        Some(left[1].to_string())
    } else {
        None
    };

    let new_path = right[0].to_string();
    let new_version = if right.len() >= 2 {
        Some(right[1].to_string())
    } else {
        None
    };

    Some(Replace {
        old_path,
        old_version,
        new_path,
        new_version,
    })
}

/// Parses all `exclude` directives from go.mod content.
fn parse_excludes(content: &str) -> Vec<Exclude> {
    let mut excludes = Vec::new();
    let mut in_block = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("exclude (") || trimmed == "exclude (" {
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
            if let Some(e) = parse_exclude_line(trimmed) {
                excludes.push(e);
            }
            continue;
        }

        if trimmed.starts_with("exclude ") && !trimmed.contains('(') {
            let rest = trimmed.strip_prefix("exclude ").unwrap().trim();
            if let Some(e) = parse_exclude_line(rest) {
                excludes.push(e);
            }
        }
    }

    excludes
}

fn parse_exclude_line(line: &str) -> Option<Exclude> {
    let line = line.split("//").next()?.trim();
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() == 2 {
        Some(Exclude {
            module_path: parts[0].to_string(),
            version: parts[1].to_string(),
        })
    } else {
        None
    }
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

    fn test_resolver(module_path: &str, requires: Vec<Require>, replaces: Vec<Replace>, excludes: Vec<Exclude>) -> ModuleResolver {
        ModuleResolver {
            module_path: module_path.to_string(),
            project_root: PathBuf::from("/project"),
            requires,
            replaces,
            excludes,
            cache_dir: PathBuf::from("/tmp/cache"),
            go_sum_path: PathBuf::from("/project/go.sum"),
        }
    }

    #[test]
    fn test_resolve_import_subpackage() {
        let resolver = test_resolver("github.com/gnolang/gno-rs/local_importing", vec![], vec![], vec![]);
        let result = resolver.resolve_import("github.com/gnolang/gno-rs/local_importing/add");
        assert_eq!(result.unwrap(), PathBuf::from("/project/add"));
    }

    #[test]
    fn test_resolve_import_root() {
        let resolver = test_resolver("github.com/gnolang/gno-rs/local_importing", vec![], vec![], vec![]);
        let result = resolver.resolve_import("github.com/gnolang/gno-rs/local_importing");
        assert_eq!(result.unwrap(), PathBuf::from("/project"));
    }

    #[test]
    fn test_resolve_import_outside_module() {
        let resolver = test_resolver("github.com/gnolang/gno-rs/local_importing", vec![], vec![], vec![]);
        let result = resolver.resolve_import("github.com/other/pkg");
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_require_match() {
        let resolver = test_resolver("example.com/mymod", vec![Require {
            module_path: "github.com/foo/bar".to_string(),
            version: "v1.0.0".to_string(),
        }], vec![], vec![]);
        let req = resolver.find_require("github.com/foo/bar/pkg/sub");
        assert!(req.is_some());
        assert_eq!(req.unwrap().module_path, "github.com/foo/bar");
    }

    #[test]
    fn test_cached_module_path() {
        let resolver = test_resolver("example.com/mymod", vec![], vec![], vec![]);
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
        let resolver = test_resolver("example.com/mymod", vec![
            Require {
                module_path: "github.com/foo".to_string(),
                version: "v1.0.0".to_string(),
            },
            Require {
                module_path: "github.com/foo/bar".to_string(),
                version: "v2.0.0".to_string(),
            },
        ], vec![], vec![]);
        let req = resolver.find_require("github.com/foo/bar/baz");
        assert!(req.is_some());
        assert_eq!(req.unwrap().module_path, "github.com/foo/bar");
        assert_eq!(req.unwrap().version, "v2.0.0");
    }

    #[test]
    fn test_find_require_no_match() {
        let resolver = test_resolver("example.com/mymod", vec![Require {
            module_path: "github.com/foo/bar".to_string(),
            version: "v1.0.0".to_string(),
        }], vec![], vec![]);
        let req = resolver.find_require("github.com/other/thing");
        assert!(req.is_none());
    }

    #[test]
    fn test_resolve_import_no_require_returns_error() {
        let resolver = test_resolver("example.com/mymod", vec![], vec![], vec![]);
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
        let resolver = test_resolver("example.com/mymod", vec![], vec![], vec![]);
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

    // --- Replace directive tests ---

    #[test]
    fn test_parse_replaces_single_local() {
        let content = "module example.com/foo\nreplace github.com/bar/baz => ../local/baz\n";
        let repls = parse_replaces(content);
        assert_eq!(repls.len(), 1);
        assert_eq!(repls[0].old_path, "github.com/bar/baz");
        assert!(repls[0].old_version.is_none());
        assert_eq!(repls[0].new_path, "../local/baz");
        assert!(repls[0].new_version.is_none());
    }

    #[test]
    fn test_parse_replaces_single_versioned() {
        let content = "module example.com/foo\nreplace github.com/bar/baz v1.0.0 => github.com/fork/baz v1.0.1\n";
        let repls = parse_replaces(content);
        assert_eq!(repls.len(), 1);
        assert_eq!(repls[0].old_path, "github.com/bar/baz");
        assert_eq!(repls[0].old_version.as_deref(), Some("v1.0.0"));
        assert_eq!(repls[0].new_path, "github.com/fork/baz");
        assert_eq!(repls[0].new_version.as_deref(), Some("v1.0.1"));
    }

    #[test]
    fn test_parse_replaces_block() {
        let content = "\
module example.com/foo

replace (
\tgithub.com/a/b => ../local_a
\tgithub.com/c/d v1.0.0 => github.com/fork/d v1.1.0
)
";
        let repls = parse_replaces(content);
        assert_eq!(repls.len(), 2);
        assert_eq!(repls[0].old_path, "github.com/a/b");
        assert_eq!(repls[0].new_path, "../local_a");
        assert_eq!(repls[1].old_path, "github.com/c/d");
        assert_eq!(repls[1].new_path, "github.com/fork/d");
    }

    #[test]
    fn test_resolve_import_with_local_replace() {
        let resolver = test_resolver(
            "example.com/mymod",
            vec![],
            vec![Replace {
                old_path: "github.com/other/mod".to_string(),
                old_version: None,
                new_path: "../sibling".to_string(),
                new_version: None,
            }],
            vec![],
        );
        let result = resolver.resolve_import("github.com/other/mod/pkg");
        assert_eq!(result.unwrap(), PathBuf::from("/project/../sibling/pkg"));
    }

    #[test]
    fn test_resolve_import_with_replace_root() {
        let resolver = test_resolver(
            "example.com/mymod",
            vec![],
            vec![Replace {
                old_path: "github.com/other/mod".to_string(),
                old_version: None,
                new_path: "./local".to_string(),
                new_version: None,
            }],
            vec![],
        );
        let result = resolver.resolve_import("github.com/other/mod");
        assert_eq!(result.unwrap(), PathBuf::from("/project/./local"));
    }

    // --- Exclude directive tests ---

    #[test]
    fn test_parse_excludes_single() {
        let content = "module example.com/foo\nexclude github.com/bar/baz v1.0.0\n";
        let excls = parse_excludes(content);
        assert_eq!(excls.len(), 1);
        assert_eq!(excls[0].module_path, "github.com/bar/baz");
        assert_eq!(excls[0].version, "v1.0.0");
    }

    #[test]
    fn test_parse_excludes_block() {
        let content = "\
module example.com/foo

exclude (
\tgithub.com/a/b v1.0.0
\tgithub.com/c/d v2.0.0
)
";
        let excls = parse_excludes(content);
        assert_eq!(excls.len(), 2);
        assert_eq!(excls[0].module_path, "github.com/a/b");
        assert_eq!(excls[0].version, "v1.0.0");
        assert_eq!(excls[1].module_path, "github.com/c/d");
        assert_eq!(excls[1].version, "v2.0.0");
    }

    #[test]
    fn test_is_excluded() {
        let resolver = test_resolver(
            "example.com/mymod",
            vec![Require {
                module_path: "github.com/foo/bar".to_string(),
                version: "v1.0.0".to_string(),
            }],
            vec![],
            vec![Exclude {
                module_path: "github.com/foo/bar".to_string(),
                version: "v1.0.0".to_string(),
            }],
        );
        assert!(resolver.is_excluded("github.com/foo/bar", "v1.0.0"));
        assert!(!resolver.is_excluded("github.com/foo/bar", "v2.0.0"));
        assert!(!resolver.is_excluded("github.com/other/pkg", "v1.0.0"));
    }

    #[test]
    fn test_resolve_import_excluded() {
        let resolver = test_resolver(
            "example.com/mymod",
            vec![Require {
                module_path: "github.com/foo/bar".to_string(),
                version: "v1.0.0".to_string(),
            }],
            vec![],
            vec![Exclude {
                module_path: "github.com/foo/bar".to_string(),
                version: "v1.0.0".to_string(),
            }],
        );
        let result = resolver.resolve_import("github.com/foo/bar");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("excluded"));
    }

    #[test]
    fn test_replace_takes_priority_over_local() {
        let resolver = test_resolver(
            "example.com/mymod",
            vec![],
            vec![Replace {
                old_path: "example.com/mymod/sub".to_string(),
                old_version: None,
                new_path: "../other".to_string(),
                new_version: None,
            }],
            vec![],
        );
        let result = resolver.resolve_import("example.com/mymod/sub");
        assert_eq!(result.unwrap(), PathBuf::from("/project/../other"));
    }

    #[test]
    fn test_from_project_root_with_replace() {
        let dir = std::env::temp_dir().join("govm_test_replace");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("go.mod"),
            "module example.com/testmod\n\nreplace github.com/dep/a => ../local_dep\n",
        )
        .unwrap();

        let resolver = ModuleResolver::from_project_root(&dir);
        assert!(resolver.is_some());
        let resolver = resolver.unwrap();
        assert_eq!(resolver.replaces.len(), 1);
        assert_eq!(resolver.replaces[0].old_path, "github.com/dep/a");
        assert_eq!(resolver.replaces[0].new_path, "../local_dep");

        let _ = fs::remove_dir_all(&dir);
    }

    // --- go.sum tests ---

    #[test]
    fn test_hash_dir() {
        let dir = std::env::temp_dir().join("govm_test_hash_dir");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.go"), "package main\n").unwrap();
        fs::write(dir.join("b.go"), "package main\nfunc B() {}\n").unwrap();

        let hash = hash_dir(&dir);
        assert!(hash.is_ok());
        let hash = hash.unwrap();
        assert!(hash.starts_with("h1:"));

        // Same content should produce the same hash
        let hash2 = hash_dir(&dir).unwrap();
        assert_eq!(hash, hash2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_hash_dir_changes_on_modification() {
        let dir = std::env::temp_dir().join("govm_test_hash_mod");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.go"), "package main\n").unwrap();

        let hash1 = hash_dir(&dir).unwrap();

        fs::write(dir.join("a.go"), "package main\nfunc A() {}\n").unwrap();

        let hash2 = hash_dir(&dir).unwrap();
        assert_ne!(hash1, hash2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_write_go_sum() {
        let dir = std::env::temp_dir().join("govm_test_gosum");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let sum_path = dir.join("go.sum");

        let mut entries = BTreeMap::new();
        entries.insert("github.com/foo/bar v1.0.0".to_string(), "h1:abc123==".to_string());
        entries.insert("github.com/baz/qux v2.0.0".to_string(), "h1:def456==".to_string());

        write_go_sum(&sum_path, &entries).unwrap();

        let read_back = read_go_sum(&sum_path);
        assert_eq!(read_back.len(), 2);
        assert_eq!(read_back.get("github.com/foo/bar v1.0.0").unwrap(), "h1:abc123==");
        assert_eq!(read_back.get("github.com/baz/qux v2.0.0").unwrap(), "h1:def456==");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_go_sum_empty() {
        let entries = read_go_sum(Path::new("/nonexistent/go.sum"));
        assert!(entries.is_empty());
    }

    #[test]
    fn test_verify_module_no_gosum() {
        let resolver = test_resolver("example.com/mymod", vec![], vec![], vec![]);
        // No go.sum file => verification passes (nothing to check)
        assert!(resolver.verify_module("github.com/foo/bar", "v1.0.0").is_ok());
    }
}
