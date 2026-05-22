fn main() {
    // Lock the std-widgets style to "material" so widget chrome (LineEdit /
    // ScrollView / TextEdit focus rings, scrollbar handles, input borders)
    // matches the Material 3 dark theme the rest of the app paints from the
    // Theme global. Default style is platform-dependent — explicit pin keeps
    // appearance deterministic across macOS / Linux / Windows builds.
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new().with_style("material".into()),
    )
    .expect("failed to compile ui/app.slint");
}
