use std::fs;
use std::process::Command;

const SKIP_TESTS: &[&str] = &["foreign_importing"];

#[test]
fn compiler_ui_tests() {
    let ui_tests_dir = "./tests/ui";
    let compiler_command = "cargo";

    for entry in fs::read_dir(ui_tests_dir).expect("Failed to read UI tests directory") {
        if let Ok(entry) = entry {
            let project_path = entry.path();
            assert!(project_path.is_dir());

            let dir_name = project_path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap();
            if SKIP_TESTS.contains(&dir_name) {
                continue;
            }

            let main_file = project_path.join("main.go");
            assert!(
                main_file.exists(),
                "Test directory {:?} is missing main.go",
                project_path
            );

            let status = Command::new(compiler_command)
                .arg("run")
                .arg("run")
                .arg("--output-assert")
                .arg(main_file.to_str().unwrap())
                .status()
                .expect("Failed to execute compiler");

            assert!(status.success(), "Test project failed: {:?}", project_path);
        }
    }
}
