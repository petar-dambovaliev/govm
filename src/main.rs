pub mod parser;
pub mod stdlib;
pub mod vm;
pub mod wasm;

use crate::vm::compiler::compiler::Compiler;
use crate::vm::module::parse_local_dependencies;
use clap::Args;
use clap::{Parser as ClapParser, Subcommand};
use gno_rs::gomod::{add_dependency, list_dependencies, remove_dependency};
use std::env;
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use wasmtime::{Caller, Engine, Linker, Module, Store};

#[derive(ClapParser, Debug)]
struct Cli {
    #[clap(short, long)]
    debug: Option<u8>,

    #[clap(subcommand)]
    action: SubCommand,
}

#[derive(Subcommand, Debug)]
enum SubCommand {
    #[clap(name = "build", about = "Compile Go source to WASM")]
    Build {
        source: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    #[clap(name = "run", about = "Builds and runs a Go binary via Wasmtime")]
    Run {
        binary: PathBuf,
    },
    #[clap(name = "mod", about = "Manage Go modules")]
    Mod(ModArg),
}

#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true)]
struct ModArg {
    #[command(subcommand)]
    command: ModSubCommand,
}

#[derive(Debug, Subcommand)]
enum ModSubCommand {
    #[clap(name = "init", about = "Initialize a new module")]
    Init { module_name: Option<String> },
    #[clap(name = "add", about = "Add a new dependency")]
    Add { dependency: String },
    #[clap(name = "list", about = "List dependencies")]
    List,
    #[clap(name = "remove", about = "Remove a dependency")]
    Remove { dependency: String },
}

struct HostState {
    output: Vec<u8>,
}

fn host_print_string(mut caller: Caller<'_, HostState>, ptr: i32, len: i32) {
    if len <= 0 {
        return;
    }
    let mem = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .expect("memory export");
    let data = mem.data(&caller);
    let start = ptr as usize;
    let end = start + len as usize;
    if end <= data.len() {
        let bytes = &data[start..end];
        use std::io::Write;
        let _ = std::io::stdout().write_all(bytes);
        let _ = std::io::stdout().flush();
    }
}

fn host_println_string(mut caller: Caller<'_, HostState>, ptr: i32, len: i32) {
    let mem = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .expect("memory export");
    let data = mem.data(&caller);
    let mut bytes = Vec::new();
    if len > 0 {
        let start = ptr as usize;
        let end = start + len as usize;
        if end <= data.len() {
            bytes.extend_from_slice(&data[start..end]);
        }
    }
    bytes.push(b'\n');
    use std::io::Write;
    let _ = std::io::stdout().write_all(&bytes);
    let _ = std::io::stdout().flush();
}

fn compile_to_wasm(source: &PathBuf) -> Vec<u8> {
    let mut src = source.clone();
    let pkgs = parse_local_dependencies(&src).unwrap();

    let mut goc = Compiler::new();
    let main = src.clone();
    src.pop();

    goc.compile(main, src, pkgs, false).unwrap()
}

fn run_wasm(wasm_bytes: &[u8]) -> Result<(), String> {
    let engine = Engine::default();
    let module = Module::new(&engine, wasm_bytes).map_err(|e| e.to_string())?;

    let mut linker = Linker::new(&engine);
    linker
        .func_wrap("env", "rt_gc_collect", |_caller: Caller<'_, HostState>| {})
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap("env", "print_string", host_print_string)
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap("env", "println_string", host_println_string)
        .map_err(|e| e.to_string())?;

    let mut store = Store::new(
        &engine,
        HostState {
            output: Vec::new(),
        },
    );

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| e.to_string())?;

    let main_fn = instance
        .get_func(&mut store, "main")
        .ok_or_else(|| "missing export 'main'".to_string())?;

    let ty = main_fn.ty(&store);
    let mut results = vec![wasmtime::Val::I32(0); ty.results().len()];

    main_fn
        .call(&mut store, &[], &mut results)
        .map_err(|e| e.to_string())?;

    if !results.is_empty() {
        if let Some(wasmtime::Val::I32(v)) = results.first() {
            std::process::exit(*v);
        }
    }

    Ok(())
}

fn main() {
    let cli = Cli::parse();

    match cli.action {
        SubCommand::Build { source, output } => {
            let wasm_bytes = compile_to_wasm(&source);

            let out_path = output.unwrap_or_else(|| {
                let mut p = source.clone();
                p.set_extension("wasm");
                p
            });

            let mut file = File::create(&out_path)
                .unwrap_or_else(|e| panic!("failed to create output file {:?}: {}", out_path, e));
            file.write_all(&wasm_bytes)
                .unwrap_or_else(|e| panic!("failed to write WASM: {}", e));

            println!("compiled to {:?}", out_path);
        }
        SubCommand::Run { mut binary } => {
            let is_wasm = binary.extension().map_or(false, |ext| ext == "wasm");

            let wasm_bytes = if is_wasm {
                let mut file = File::open(&binary)
                    .unwrap_or_else(|e| panic!("failed to open {:?}: {}", binary, e));
                let mut data = Vec::new();
                file.read_to_end(&mut data)
                    .unwrap_or_else(|e| panic!("failed to read {:?}: {}", binary, e));
                data
            } else {
                let pkgs = parse_local_dependencies(&binary).unwrap();
                let mut goc = Compiler::new();
                let main = binary.clone();
                binary.pop();
                goc.compile(main, binary, pkgs, false).unwrap()
            };

            if let Err(e) = run_wasm(&wasm_bytes) {
                eprintln!("{}", e);
                std::process::exit(1);
            }
        }
        SubCommand::Mod(mod_command) => match mod_command.command {
            ModSubCommand::Init { module_name } => {
                let name = module_name.unwrap_or_else(|| {
                    env::current_dir()
                        .ok()
                        .as_ref()
                        .and_then(|cd| cd.file_name())
                        .and_then(|name| name.to_str())
                        .map(|a| a.to_string())
                        .expect("no module name")
                });

                let go_mod_content = format!("module {}\n", name);

                if let Ok(cd) = env::current_dir() {
                    let go_mod_path = cd.join("go.mod");
                    let mut file = File::create(go_mod_path).expect("Failed to create go.mod");
                    file.write_all(go_mod_content.as_bytes())
                        .expect("Failed to write to go.mod");
                } else {
                    panic!("Failed to get the current directory.");
                }
            }
            ModSubCommand::Add { dependency } => {
                add_dependency(&dependency).unwrap();
            }
            ModSubCommand::List => {
                list_dependencies().unwrap();
            }
            ModSubCommand::Remove { dependency } => {
                remove_dependency(&dependency).unwrap();
            }
        },
    }
}
