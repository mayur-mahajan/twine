//! `twine i18n`: the command-line front end writes what `twine_build::i18n` generates.

use std::path::PathBuf;

use twine_cli::i18n::{I18nArgs, run, translations};

fn args(out: PathBuf) -> I18nArgs {
    I18nArgs {
        inputs: vec![PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/twine-extra/tests/i18n"
        ))],
        out,
        default: Some("en".into()),
        languages: ["en", "de", "ru", "ar", "zh", "fr", "hi", "fa", "ja"]
            .map(String::from)
            .to_vec(),
        font: vec!["ar=::twine_text::EMPTY_FONT".into()],
        dir: Vec::new(),
        plural: Vec::new(),
        fallback: Vec::new(),
        strict: false,
        runtime: None,
    }
}

#[test]
fn writes_the_committed_test_catalog() {
    let out = std::env::temp_dir().join(format!("twine-cli-i18n-{}.rs", std::process::id()));
    run(&args(out.clone())).unwrap();
    let written = std::fs::read_to_string(&out).unwrap();
    let _ = std::fs::remove_file(&out);
    let committed = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/twine-extra/tests/i18n/catalog.rs"
    ))
    .unwrap();
    assert!(written == committed, "the CLI and twine-build disagree");
}

#[test]
fn bad_options_and_strict_mode_are_errors() {
    let mut a = args(PathBuf::from("unused.rs"));
    a.dir = vec!["ar=sideways".into()];
    assert!(translations(&a).unwrap_err().contains("expected `ltr` or `rtl`"));
    let mut a = args(PathBuf::from("unused.rs"));
    a.font = vec!["no-equals".into()];
    assert!(
        translations(&a)
            .unwrap_err()
            .contains("expected `<language>=<value>`")
    );
    let mut a = args(PathBuf::from("unused.rs"));
    a.strict = true;
    let e = translations(&a).unwrap().build().unwrap_err();
    assert!(e.to_string().contains("missing translation"), "{e}");
}
