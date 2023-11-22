mod semserver;

use std::collections::HashMap;
use std::fs;

use git2::build::RepoBuilder;
use git2::{FetchOptions, Repository};
use regex::Regex;
use reqwest;
use std::env;
use std::error::Error;
use std::fs::File;
use std::io;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

pub fn remove_dependency(module_to_remove: &str) -> Result<(), Box<dyn Error>> {
    // Read the contents of the go.mod file
    let go_mod_path = "go.mod";
    let mut go_mod_contents = fs::read_to_string(go_mod_path)?;

    // Define a regular expression to match require lines
    let re = Regex::new(r#"^\s*require\s+"([^"]+)""#)?;

    // Open the go.mod file for writing
    let mut go_mod_file = fs::File::create(go_mod_path)?;

    // Iterate through lines in the go.mod file
    let mut is_dependency_removed = false;
    for line in go_mod_contents.lines() {
        if let Some(captures) = re.captures(line) {
            let module_name = captures.get(1).unwrap().as_str();
            if module_name != module_to_remove {
                writeln!(go_mod_file, "require \"{}\"", module_name)?;
            } else {
                is_dependency_removed = true;
            }
        } else {
            writeln!(go_mod_file, "{}", line)?;
        }
    }

    // Close the go.mod file
    go_mod_file.flush()?;

    if is_dependency_removed {
        println!("Removed dependency: {}", module_to_remove);
    } else {
        eprintln!("Dependency not found: {}", module_to_remove);
    }

    Ok(())
}

pub fn list_dependencies() -> Result<(), Box<dyn Error>> {
    let go_mod_contents = fs::read_to_string("go.mod")?;
    let re = Regex::new(r#"^\s*module\s+"([^"]+)""#)?;

    for line in go_mod_contents.lines() {
        if let Some(captures) = re.captures(line) {
            let module_name = captures.get(1).unwrap().as_str();
            println!("{}", module_name);
        }
    }

    Ok(())
}

pub fn add_dependency(dependency: &str) -> Result<(), String> {
    // Run 'go get' to fetch the new dependency
    let go_get_result = download_dependency(dependency, "");

    if go_get_result.is_ok() {
        let go_mod_path = env::current_dir()
            .ok()
            .map(|cd| cd.join("go.mod"))
            .expect("Failed to get current directory.");

        let mut file_lines: Vec<String> = match File::open(&go_mod_path) {
            Ok(file) => BufReader::new(file)
                .lines()
                .map(|line| line.unwrap_or_default())
                .collect(),
            Err(_) => return Err("Failed to open go.mod".to_string()),
        };

        if file_lines.iter().any(|line| line.contains(dependency)) {
            return Ok(());
        }

        if let Some((idx, line)) = file_lines
            .iter()
            .enumerate()
            .find(|(_, line)| line.starts_with("module "))
        {
            file_lines.insert(idx + 1, format!("    {}", dependency));
        } else {
            return Err("Failed to find the module line in go.mod".to_string());
        }

        if let Ok(mut file) = File::create(&go_mod_path) {
            for line in &file_lines {
                writeln!(&mut file, "{}", line).expect("Failed to write to go.mod");
            }
            println!("Added dependency: {}", dependency);
        } else {
            return Err("Failed to update go.mod".to_string());
        }
    } else {
        return Err(format!(
            "Failed to add dependency {}. Please make sure it's a valid Go package.",
            dependency
        ));
    }

    Ok(())
}

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
