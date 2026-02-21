use super::*;

#[test]
fn language_id_rust() {
    let p = std::path::Path::new("foo.rs");
    assert_eq!(language_id_from_path(p), "rust");
}

#[test]
fn language_id_go() {
    let p = std::path::Path::new("main.go");
    assert_eq!(language_id_from_path(p), "go");
}

#[test]
fn language_id_unknown() {
    let p = std::path::Path::new("readme.md");
    assert_eq!(language_id_from_path(p), "plaintext");
}

#[test]
fn path_to_uri_absolute() {
    let path = std::path::Path::new("/tmp/foo.rs");
    assert_eq!(path_to_uri(path).unwrap(), "file:///tmp/foo.rs");
}

#[test]
fn symbol_kind_function() {
    assert_eq!(symbol_kind_name(12), "Function");
}

#[test]
fn symbol_kind_unknown() {
    assert_eq!(symbol_kind_name(999), "Unknown");
}
