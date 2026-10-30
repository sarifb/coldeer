//! Stable cache partitions: platform tools, application roots/dependencies, extras.

use crate::json::Value as V;

fn op(name: &str, args: impl IntoIterator<Item = V>) -> V {
    V::Array(std::iter::once(V::from(name)).chain(args).collect())
}

fn pipe(steps: Vec<V>) -> V {
    op("pipe", [V::Array(steps)])
}

fn over(key: &str, operation: V) -> V {
    op("over", [key.into(), operation])
}

fn ranked(limit: usize) -> V {
    pipe(vec![
        op("popularity_contest", []),
        op("reverse", []),
        op("limit_layers", [V::Number(limit.to_string())]),
    ])
}

/// Nix prepends the platform-tools partition because only it knows those paths.
/// Every application root gets a separate layer; the two dependency populations
/// have independent budgets, so adding diagnostics cannot repartition app deps.
pub fn pipeline(main: &[String]) -> V {
    let budget = 125usize.saturating_sub(main.len()).max(2);
    let application = 90.min(budget - 1);
    let extras = 30.min(budget - application);
    V::Array(vec![over(
        "rest",
        pipe(vec![
            op("subcomponent_out", [V::array(main.iter().cloned())]),
            over(
                "main",
                pipe(vec![
                    op("subcomponent_in", [V::array(main.iter().cloned())]),
                    over("main", op("split_every", [V::Number("1".into())])),
                    over("rest", ranked(application)),
                ]),
            ),
            over("rest", ranked(extras)),
        ]),
    )])
}
