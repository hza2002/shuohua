fn main() {
    println!("cargo:rerun-if-changed=src/asr/providers/apple_helper.swift");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=ApplicationServices");
    println!("cargo:rustc-link-lib=framework=QuartzCore");

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR set by cargo");
    let helper_out = std::path::Path::new(&out_dir).join("apple_helper");
    let status = std::process::Command::new("xcrun")
        .args([
            "swiftc",
            "-O",
            "-parse-as-library",
            "-o",
            helper_out.to_str().expect("helper path is utf-8"),
            "src/asr/providers/apple_helper.swift",
        ])
        .status()
        .expect("run xcrun swiftc for apple_helper");
    if !status.success() {
        panic!("swiftc failed building apple_helper");
    }
}
