//! CLI-R8: without the `record` feature the subcommand still parses and reports the feature.
#![cfg(not(feature = "record"))]

#[path = "common/hermetic.rs"]
mod hermetic;

#[test]
fn cli_r8_record_without_the_feature_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("x.gif");
    let output = hermetic::sciart()
        .args(["record", "aurora", "--out", out.to_str().unwrap()])
        .output()
        .expect("spawn the sciart binary");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("record"), "{stderr}");
    assert!(stderr.contains("feature"), "{stderr}");
    assert!(!out.exists(), "nothing is written without the feature");
}
