//! pack/unpack: директория <-> один .eml, path traversal protection.

use emlbox::pack;
use emlbox::writer::{build_file, Part};
use std::path::PathBuf;

fn setup(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("emlbox_pack_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn pack_unpack_roundtrip_binary_and_text() {
    let root = setup("rt");
    std::fs::create_dir_all(root.join("sub/deep")).unwrap();
    std::fs::write(root.join("a.txt"), b"hello world").unwrap();
    std::fs::write(root.join("sub/b.json"), br#"{"k":1}"#).unwrap();
    // binary with all 256 byte values
    let bin: Vec<u8> = (0..=255u8).collect();
    std::fs::write(root.join("sub/deep/c.bin"), &bin).unwrap();

    let out = root.with_file_name("pkg.eml");
    let (n, _) = pack::pack(&root, &out, "pkg@system.local").unwrap();
    assert_eq!(n, 3);

    // verify container integrity
    assert!(emlbox::verify::verify(&out).unwrap().is_empty());

    let dest = root.join("out");
    let n = pack::unpack(&out, &dest).unwrap();
    assert_eq!(n, 3);

    assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"hello world");
    assert_eq!(std::fs::read(dest.join("sub/b.json")).unwrap(), br#"{"k":1}"#);
    assert_eq!(std::fs::read(dest.join("sub/deep/c.bin")).unwrap(), bin);
}

#[test]
fn pack_preserves_relative_paths_in_sections() {
    let root = setup("paths");
    std::fs::create_dir_all(root.join("x/y")).unwrap();
    std::fs::write(root.join("x/y/f.py"), b"print(1)").unwrap();
    let out = root.with_file_name("p.eml");
    pack::pack(&root, &out, "e").unwrap();

    let b = emlbox::reader::EmlBox::open(&out).unwrap();
    assert_eq!(b.sections.len(), 1);
    assert_eq!(b.sections[0].id, "x/y/f.py");
    assert_eq!(b.sections[0].ct, "text/x-python");
    assert_eq!(b.section("x/y/f.py").unwrap(), b"print(1)");
}

#[test]
fn unpack_rejects_path_traversal() {
    let root = setup("traversal");
    // craft a malicious container: section id = ../../evil
    let out = root.join("evil.eml");
    build_file(
        &out,
        "evil@system.local",
        "evil",
        vec![Part::raw("../../evil", "text/plain", "../../evil", b"pwn".to_vec())],
    )
    .unwrap();
    let dest = root.join("out");
    let r = pack::unpack(&out, &dest);
    assert!(r.is_err(), "path traversal must be rejected");
    assert!(!dest.join("..").join("..").join("evil").exists());
}

#[test]
fn unpack_rejects_absolute_path() {
    let root = setup("abs");
    let out = root.join("abs.eml");
    let id = "/tmp/emlbox_pack_evil_abs".to_string();
    build_file(
        &out,
        "e",
        "e",
        vec![Part::raw(&id, "text/plain", &id, b"x".to_vec())],
    )
    .unwrap();
    assert!(pack::unpack(&out, &root.join("out")).is_err());
}

#[test]
fn empty_dir_packs_to_empty_container() {
    let root = setup("empty");
    let out = root.with_file_name("empty.eml");
    let (n, bytes) = pack::pack(&root, &out, "e").unwrap();
    assert_eq!(n, 0);
    assert_eq!(bytes, 0);
    let b = emlbox::reader::EmlBox::open(&out).unwrap();
    assert_eq!(b.sections.len(), 0);
}
