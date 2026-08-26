fn main() {
    // Windows: cargo's test binaries get no application manifest (only the
    // [[bin]] does, via tauri-build). Without one the loader activates the
    // legacy v5.82 Common-Controls assembly, whose comctl32.dll fails to bind
    // `CoTaskMemAlloc` (api-ms-win-core-com-l1-1-0) at load ->
    // STATUS_ENTRYPOINT_NOT_FOUND, crashing the test binary before a single
    // test runs (tauri -> wry -> comctl32 links into every target).
    //
    // `-tests` covers the tests/ integration binaries (TargetKind::Test) and
    // never touches the [[bin]], so this is safe for `cargo build`/release. The
    // `--lib` unit-test harness (TargetKind::Lib in test mode) is reachable only
    // by the unscoped `rustc-link-arg`, which collides with tauri's manifest on
    // the bin (CVT1100/LNK1123) - so it is injected via RUSTFLAGS from
    // scripts/test-windows.ps1 instead. Two classes, two mechanisms, two passes.
    //
    // Gated on CARGO_CFG_TARGET_OS (the TARGET, not the host) so a cross-build
    // to Windows from elsewhere gets it too.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let manifest = format!(
            "{}/windows-test.manifest",
            std::env::var("CARGO_MANIFEST_DIR").unwrap()
        );
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-tests=/MANIFESTINPUT:{manifest}");
        println!("cargo:rerun-if-changed=windows-test.manifest");
    }

    tauri_build::build();
}
