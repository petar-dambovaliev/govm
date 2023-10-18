use crate::parser::ast::{File, Package};
use crate::parser::{parse_dir_recursive, Error, Result};
use std::path::{Path, PathBuf};

pub fn parse_dependencies(f: &PathBuf) -> Result<Vec<Package>> {
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

    // for import in &f.imports {
    //     //import.name alias
    //     // is local
    //     if import.path.value.starts_with('.') {
    //         let path = Path::new(import.path.value.as_str());
    //         if !path.is_dir() {
    //             panic!("cannot import a file")
    //         }
    //
    //         let pkg = local_pkgs
    //             .iter()
    //             .find(|p| p.path.canonicalize().unwrap() == path.canonicalize().unwrap())
    //             .expect(&format!(
    //                 "cannot find package {}",
    //                 path.canonicalize().unwrap().to_str().unwrap()
    //             ));
    //     } else {
    //         unimplemented!("remote dependencies");
    //     }
    // }

    Ok(local_pkgs)
}
