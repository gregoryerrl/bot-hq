//! Smoke-test surface for the tauri-specta + Tauri v2 pipeline.
//!
//! Returns a `Builder` with the current command set. Batches 2+ add real
//! commands via `.commands(collect_commands![...])`. Batch 0 ships with an
//! empty command set so the toolchain (cargo + tauri-build + tauri-specta +
//! specta-typescript) is exercised before downstream wrappers depend on it.

use tauri_specta::Builder;

pub fn builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_constructs_with_empty_commands() {
        let _b = builder();
    }

    #[test]
    fn builder_exports_to_typescript_stub() {
        let b = builder();
        let out = std::env::temp_dir().join("bot-hq-types-smoke.ts");
        b.export(specta_typescript::Typescript::default(), &out)
            .expect("tauri-specta export must succeed for empty command set");
        assert!(out.exists());
    }
}
