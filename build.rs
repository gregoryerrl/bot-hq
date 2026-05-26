fn main() {
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new().with_style("material".into()),
    )
    .expect("failed to compile ui/app.slint");

    tauri_build::build();
}
