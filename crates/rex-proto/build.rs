//! Compiles every proto under `<repo>/proto` with prost-build and writes a FileDescriptorSet
//! (with imports, so well-known types and descriptor.proto are included) for runtime reflection.

use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let proto_root = manifest_dir
        .join("../../proto")
        .canonicalize()
        .expect("proto/ directory");
    let mut protos = Vec::new();
    collect_protos(&proto_root, &mut protos);
    protos.sort();
    for p in &protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    prost_build::Config::new()
        .file_descriptor_set_path(out_dir.join("rex.fds"))
        .compile_protos(&protos, &[proto_root])
        .expect("compile rex protos");
}

fn collect_protos(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    // Watch every directory too, so a newly added proto (or package directory) triggers a rebuild.
    println!("cargo:rerun-if-changed={}", dir.display());
    for entry in std::fs::read_dir(dir).expect("read proto dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_protos(&path, out);
        } else if path.extension().is_some_and(|e| e == "proto") {
            out.push(path);
        }
    }
}
