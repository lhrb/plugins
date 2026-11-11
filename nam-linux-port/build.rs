fn main() {
    println!("cargo:rerun-if-changed=native");
    println!("cargo:rerun-if-changed=vendor/NeuralAmpModelerCore");
    let dst = cmake::Config::new("native").profile("Release").build();
    println!("cargo:rustc-link-search=native={}/lib", dst.display());
    // Upstream architectures register through static initializers in separate objects.
    println!("cargo:rustc-link-lib=static:+whole-archive=nam_bridge");
    println!("cargo:rustc-link-lib=dylib=stdc++");
}
