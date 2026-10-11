use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const MARKETPLACE: &str = ".claude-plugin/marketplace.json";
const MOD_DIR: &str = "mod";
const LEFT_OUT: &[&str] = &["mod/tests", "mod/.claude-plugin/types"];

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rerun-if-changed={MARKETPLACE}");
    println!("cargo:rerun-if-changed={MOD_DIR}");
    let mut files = vec![MARKETPLACE.to_string()];
    collect(&root, &root.join(MOD_DIR), &mut files);
    files.sort();
    let mut source = String::from("pub const FILES: &[(&str, &[u8])] = &[\n");
    for file in &files {
        let absolute = root.join(file);
        writeln!(
            source,
            "    ({file:?}, include_bytes!({:?})),",
            absolute.display().to_string()
        )
        .unwrap();
    }
    source.push_str("];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("bundle.rs");
    std::fs::write(out, source).unwrap();
}

fn collect(root: &Path, dir: &Path, files: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let name = entry.file_name();
        let hidden = name
            .to_str()
            .is_some_and(|n| n.starts_with('.') && n != ".claude-plugin");
        if hidden || LEFT_OUT.contains(&relative.as_str()) {
            continue;
        }
        if entry.file_type().unwrap().is_dir() {
            collect(root, &path, files);
        } else {
            files.push(relative);
        }
    }
}
