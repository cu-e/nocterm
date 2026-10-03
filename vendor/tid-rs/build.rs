fn main() {
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=LocalAuthentication");
    println!("cargo:rerun-if-changed=foreign/tid.m");
    println!("cargo:rerun-if-changed=foreign/tid.h");
    cc::Build::new()
        .file("foreign/tid.m")
        .include("foreign")
        .flag("-fblocks")
        .flag("-fno-objc-arc")
        .compile("nocterm_tid");
}
