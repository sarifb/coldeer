//! Executes Nixpkgs' real graph partitioner on controlled dependency graphs.

use coldeer::{
    json::{self, Value as V},
    layering,
};
use std::{
    collections::BTreeSet,
    fs,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn node(path: &str, refs: &[String], size: usize) -> V {
    V::object([
        ("path", path.into()),
        ("references", V::array(refs.iter().cloned())),
        ("narHash", "sha256:fixture".into()),
        ("narSize", V::Number(size.to_string())),
        ("closureSize", V::Number((size * 2).to_string())),
    ])
}

fn layers(graph: Vec<V>, main: &[String]) -> Vec<BTreeSet<String>> {
    let mut pipeline = vec![V::Array(vec![
        "subcomponent_out".into(),
        V::array(["platform".into()]),
    ])];
    pipeline.extend(layering::pipeline(main).list().unwrap().iter().cloned());
    let input = V::object([("graph", V::Array(graph)), ("pipeline", V::Array(pipeline))]);
    let path = std::env::temp_dir().join(format!(
        "coldeer-layers-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&path, json::encode(&input)).unwrap();
    let result = Command::new("flatten_references_graph")
        .arg(&path)
        .output()
        .expect("enter nix develop for flatten_references_graph");
    fs::remove_file(path).unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    json::parse(std::str::from_utf8(&result.stdout).unwrap())
        .unwrap()
        .list()
        .unwrap()
        .iter()
        .map(|v| v.strings().unwrap().into_iter().collect())
        .collect()
}

fn fixture(app: &str, debug: bool) -> Vec<V> {
    let mut nodes = vec![
        node("platform", &["libc".into()], 1),
        node("libc", &[], 2),
        node("liba", &["libc".into()], 3),
        node(app, &["liba".into()], 4),
        node("extra", &["libc".into()], 5),
    ];
    if debug {
        nodes.push(node("debug", &["liba".into()], 6));
    }
    nodes
}

#[test]
#[ignore = "requires Nixpkgs flatten_references_graph (provided by nix develop)"]
fn application_change_preserves_dependency_layers() {
    let before = layers(fixture("app-v1", false), &["app-v1".into()]);
    let after = layers(fixture("app-v2", false), &["app-v2".into()]);
    assert!(before.contains(&BTreeSet::from(["app-v1".into()])));
    assert!(after.contains(&BTreeSet::from(["app-v2".into()])));
    assert_eq!(
        before
            .iter()
            .filter(|l| !l.contains("app-v1"))
            .collect::<Vec<_>>(),
        after
            .iter()
            .filter(|l| !l.contains("app-v2"))
            .collect::<Vec<_>>()
    );
    let debug = layers(fixture("app-v1", true), &["app-v1".into()]);
    for stable in before.iter().filter(|l| !l.contains("extra")) {
        assert!(debug.contains(stable));
    }
    assert!(before.contains(&BTreeSet::from(["platform".into(), "libc".into()])));
}

#[test]
#[ignore = "requires Nixpkgs flatten_references_graph (provided by nix develop)"]
fn partitions_cover_all_paths_and_respect_budget() {
    let mut graph = vec![node("platform", &[], 1)];
    let dependencies = (0..200).map(|i| format!("dep-{i}")).collect::<Vec<_>>();
    for (i, dep) in dependencies.iter().enumerate() {
        graph.push(node(dep, &[], i + 2));
    }
    let main = (0..119).map(|i| format!("app-{i}")).collect::<Vec<_>>();
    for (i, app) in main.iter().enumerate() {
        graph.push(node(app, &dependencies, i + 202));
    }
    graph.push(node("extra", &[], 400));
    let output = layers(graph, &main);
    assert!(
        output.len() <= 126,
        "one layer remains for rootfs customization"
    );
    let all = output.iter().flatten().collect::<BTreeSet<_>>();
    assert_eq!(all.len(), 321);
    assert_eq!(output.iter().map(BTreeSet::len).sum::<usize>(), all.len());
    assert!(layers(fixture("app", false), &[])
        .iter()
        .flatten()
        .any(|s| s == "app"));
}
