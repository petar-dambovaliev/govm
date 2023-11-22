use std::fs;
use std::process::Command;

#[test]
fn compiler_ui_tests() {
    let ui_tests_dir = "./tests/ui";
    let compiler_command = "cargo";

    for entry in fs::read_dir(ui_tests_dir).expect("Failed to read UI tests directory") {
        if let Ok(entry) = entry {
            let project_path = entry.path();
            assert!(project_path.is_dir());

            let status = Command::new(compiler_command)
                .arg("run")
                .arg("run")
                .arg("--output-assert")
                .arg(project_path.to_str().unwrap())
                .status()
                .expect("Failed to execute compiler");

            assert!(status.success(), "Test project failed: {:?}", project_path);
        }
    }
}
