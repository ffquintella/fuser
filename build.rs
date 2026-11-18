fn main() {
    // Register rustc cfg for switching between mount implementations.
    // When fuser MSRV is updated to v1.77 or above, we should switch from 'cargo:' to 'cargo::' syntax.
    println!(
        "cargo:rustc-check-cfg=cfg(fuser_mount_impl, values(\"pure-rust\", \"libfuse2\", \"libfuse3\"))"
    );

    let target_os =
        std::env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS should be set");

    if target_os == "linux" && cfg!(not(feature = "libfuse")) {
        println!("cargo:rustc-cfg=fuser_mount_impl=\"pure-rust\"");
    } else if target_os == "macos" {
        // macOS uses FUSE-T (https://www.fuse-t.org/): an NFS-based, kext-less FUSE
        // implementation that ships a libfuse2-compatible library and a `fuse-t.pc`
        // pkg-config file. It keeps macFUSE's wire layout, hence `macfuse-4-compat`.
        pkg_config::Config::new()
            .atleast_version("1.0.0")
            .probe("fuse-t")
            .map_err(|e| {
                eprintln!(
                    "FUSE-T not found via pkg-config: {e}\n\
                     Install it with: brew install --cask macos-fuse-t/homebrew-cask/fuse-t"
                )
            })
            .unwrap();
        println!("cargo:rustc-cfg=fuser_mount_impl=\"libfuse2\"");
        println!("cargo:rustc-cfg=feature=\"macfuse-4-compat\"");
    } else {
        // First try to link with libfuse3
        if pkg_config::Config::new()
            .atleast_version("3.0.0")
            .probe("fuse3")
            .map_err(|e| eprintln!("{e}"))
            .is_ok()
        {
            println!("cargo:rustc-cfg=fuser_mount_impl=\"libfuse3\"");
        } else {
            // Fallback to libfuse
            pkg_config::Config::new()
                .atleast_version("2.6.0")
                .probe("fuse")
                .map_err(|e| eprintln!("{e}"))
                .unwrap();
            println!("cargo:rustc-cfg=fuser_mount_impl=\"libfuse2\"");
        }
    }
}
