use std::path::Path;

mod build_location {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/personal-build-fixture.rs"
    ));
}

pub(crate) use build_location::personal_build_root;

#[test]
fn fixtures_remain_inside_the_same_personal_build_root_after_staging() {
    let home = std::env::temp_dir().join("cilai-platform-fixture-home");
    let product = std::env::temp_dir().join("cilai-platform-fixture-layout");
    let source = product.join("06代码索引/cilai-workbench");
    let expected = home.join(".cilai-personal-builds/desktop");
    let registration = serde_json::json!({"schemaVersion": 1, "applicationId": "cn.cilai.personal.workbench", "buildRoot": expected, "sourceRoot": source, "projectRoot": product});
    for path in [
        source.join("crates/platform-adapter"),
        expected.join("crates/platform-adapter"),
        expected.join("runtime-source/crates/platform-adapter"),
    ] {
        assert_eq!(
            build_location::personal_build_root_for(&path, &home, &registration).unwrap(),
            expected
        );
    }
    assert!(
        build_location::personal_build_root_for(
            Path::new("relative/crates/platform-adapter"),
            &home,
            &registration
        )
        .is_err()
    );
    assert!(
        build_location::personal_build_root_for(
            &product.join("other/crates/platform-adapter"),
            &home,
            &registration
        )
        .is_err()
    );
}
