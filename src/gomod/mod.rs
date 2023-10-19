use std::collections::HashMap;
use std::fs;

use git2::build::RepoBuilder;
use git2::{FetchOptions, Repository};
use reqwest;
use std::env;
use std::error::Error;
use std::fs::File;
use std::io;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

struct Dependency {
    name: String,
    version: String,
    checksum: String,
}

fn create_go_lock_file(dependencies: Vec<Dependency>) -> io::Result<()> {
    let lockfile_name = "go.sum";
    let tmp_lockfile_name = "go.sum.tmp";

    let lockfile_content: String = dependencies
        .iter()
        .map(|dep| format!("{} {} {}\n", dep.name, dep.version, dep.checksum))
        .collect();

    let mut tmp_lockfile = File::create(tmp_lockfile_name)?;

    tmp_lockfile.write_all(lockfile_content.as_bytes())?;

    if let Err(e) = fs::rename(tmp_lockfile_name, lockfile_name) {
        fs::remove_file(tmp_lockfile_name)?;
        return Err(e);
    }

    Ok(())
}

fn download_dependency(url: &str, version: &str) -> Result<(), Box<dyn Error>> {
    let cache_dir = env::var("GOPATH")
        .map(|gopath| format!("{}/pkg/mod/cache/vcs", gopath))
        .unwrap_or_else(|_| "./pkg/mod/cache/vcs".to_string());

    let module_name = url.split("/").last().unwrap_or("");
    let destination = format!("{}/{}/@v/{}", cache_dir, module_name, version);

    if fs::metadata(&destination).is_ok() {
        return Ok(());
    }

    fs::create_dir_all(&destination)?;

    let fetch_options = FetchOptions::new();
    RepoBuilder::new()
        .fetch_options(fetch_options)
        .clone(url, destination.as_ref())?;

    Ok(())
}

#[derive(Debug)]
struct Module {
    name: String,
    version: String,
}

type DependencyMap = HashMap<String, Module>;

impl Module {
    fn new(name: &str, version: &str) -> Self {
        Module {
            name: name.to_string(),
            version: version.to_string(),
        }
    }
}

fn parse_go_mod(file_path: &str) -> Result<DependencyMap, std::io::Error> {
    let content = fs::read_to_string(file_path)?;
    let mut dependencies = DependencyMap::new();

    for line in content.lines() {
        if line.starts_with("module") {
            // Parse the module line
            let parts: Vec<&str> = line.split(' ').collect();
            if parts.len() > 1 {
                let module = parts[1];
                dependencies.insert(module.to_string(), Module::new(module, ""));
            }
        } else if line.contains(" => ") {
            // Parse dependencies
            let parts: Vec<&str> = line.split(" => ").collect();
            if parts.len() > 1 {
                let module = parts[0];
                let version = parts[1];
                if let Some(existing) = dependencies.get_mut(module) {
                    existing.version = version.to_string();
                }
            }
        }
    }

    Ok(dependencies)
}
