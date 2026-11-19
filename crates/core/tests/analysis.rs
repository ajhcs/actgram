use std::collections::{BTreeMap, BTreeSet};

use actgram_core::{ActionId, Builder, Grammar, GrammarError, Node, NodeId};
use proptest::prelude::*;

#[test]
fn validation_identifies_the_exact_bad_definition() {
    let mut builder = Builder::new();
    let root = builder.symbol("missing");
    assert_eq!(
        builder.build(root).unwrap_err(),
        GrammarError::UndefinedSymbol {
            node: root,
            name: "missing".into()
        }
    );

    let mut builder = Builder::new();
    let root = builder.symbol("loop");
    builder.rule("loop", root).unwrap();
    assert_eq!(
        builder.rule("loop", root),
        Err(GrammarError::DuplicateRule("loop".into()))
    );
    assert_eq!(
        builder.build(root).unwrap_err(),
        GrammarError::UnproductiveRule("loop".into())
    );

    let mut builder = Builder::new();
    let child = builder.action("a");
    let root = builder.repeat(child, 3, Some(2));
    assert_eq!(
        builder.build(root).unwrap_err(),
        GrammarError::BadRepeatBounds {
            node: root,
            min: 3,
            max: 2
        }
    );

    let mut builder = Builder::new();
    let invalid = NodeId::from_index(10);
    let root = builder.seq([invalid]);
    assert_eq!(
        builder.build(root).unwrap_err(),
        GrammarError::InvalidChild {
            parent: root,
            child: invalid
        }
    );

    let mut builder = Builder::new();
    let action = builder.action("a");
    let root = builder.alt([(0, action)]);
    assert_eq!(
        builder.build(root).unwrap_err(),
        GrammarError::EmptyAlternative(root)
    );
}

#[test]
fn analysis_lookup_handles_ids_outside_the_arena() {
    let mut builder = Builder::new();
    let root = builder.action("read");
    let grammar = builder.build(root).unwrap();
    let analysis = grammar.analysis();
    let known = analysis.get(root).unwrap();
    assert_eq!(known, &analysis[root]);
    assert_eq!(known.minimum_actions(), Some(1));
    assert_eq!(analysis.get(NodeId::from_index(100)), None);
}

type Words = BTreeSet<Vec<ActionId>>;

fn concatenate(left: &Words, right: &Words) -> Words {
    left.iter()
        .flat_map(|a| {
            right
                .iter()
                .map(move |b| [a.as_slice(), b.as_slice()].concat())
        })
        .collect()
}

/// Enumerate complete derivations, with small fixture-specific depth and repeat bounds.
fn enumerate(
    grammar: &Grammar,
    node: NodeId,
    depth: u8,
    memo: &mut BTreeMap<(NodeId, u8), Words>,
) -> Words {
    if depth == 0 {
        return Words::new();
    }
    if let Some(words) = memo.get(&(node, depth)) {
        return words.clone();
    }
    let words = match &grammar[node] {
        Node::Action(id) => Words::from([vec![*id]]),
        Node::Symbol(name) => enumerate(grammar, grammar.rule(name).unwrap(), depth - 1, memo),
        Node::Seq(children) => children
            .iter()
            .fold(Words::from([vec![]]), |prefix, &child| {
                concatenate(&prefix, &enumerate(grammar, child, depth - 1, memo))
            }),
        Node::Alt(arms) => arms
            .iter()
            .filter(|arm| arm.weight != 0)
            .flat_map(|arm| enumerate(grammar, arm.node, depth - 1, memo))
            .collect(),
        Node::Repeat { body, min, max } => {
            let body = enumerate(grammar, *body, depth - 1, memo);
            let mut words = Words::new();
            let mut product = Words::from([vec![]]);
            for n in 0..=max.expect("the oracle fixture uses finite repeats") {
                if n >= *min {
                    words.extend(product.iter().cloned());
                }
                product = concatenate(&product, &body);
            }
            words
        }
    };
    memo.insert((node, depth), words.clone());
    words
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, failure_persistence: None, ..ProptestConfig::default() })]
    #[test]
    fn analysis_matches_complete_derivations(
        forms in prop::array::uniform8(any::<bool>()),
        minimum in 0u32..=1,
        maximum in 1u32..=2,
    ) {
        let mut b = Builder::new();
        let a = b.action("a");
        let x = b.action("x");
        let c = b.action("c");
        let hidden = b.action("disabled");
        let empty = b.seq([]);
        let s = b.symbol("S");
        let t = b.symbol("T");
        let branch_s = b.seq([if forms[0] { a } else { empty }, t, if forms[1] { c } else { empty }]);
        let branch_t = b.seq([if forms[2] { x } else { empty }, s, if forms[3] { a } else { empty }]);
        let rule_s = b.alt([(1, if forms[4] { a } else { empty }), (1, branch_s), (0, hidden)]);
        let rule_t = b.alt([(1, if forms[5] { c } else { empty }), (1, branch_t)]);
        b.rule("S", rule_s).unwrap();
        b.rule("T", rule_t).unwrap();
        let repeated = b.repeat(if forms[6] { x } else { empty }, minimum, Some(maximum));
        let suppressed = b.repeat(hidden, 0, Some(0));
        let root = b.seq([repeated, suppressed, s, if forms[7] { c } else { empty }]);
        let grammar = b.build(root).unwrap();
        let mut memo = BTreeMap::new();
        for (node, _) in grammar.nodes() {
            let words = enumerate(&grammar, node, 16, &mut memo);
            let summary = &grammar.analysis()[node];
            prop_assert_eq!(summary.nullable(), words.contains(&vec![]));
            prop_assert_eq!(summary.minimum_actions(), words.iter().map(|word| word.len() as u64).min());
            let first: BTreeSet<_> = words.iter().filter_map(|word| word.first().copied()).collect();
            prop_assert_eq!(summary.first().collect::<BTreeSet<_>>(), first);
            let reachable: BTreeSet<_> = words.iter().flatten().copied().collect();
            prop_assert_eq!(summary.reachable().collect::<BTreeSet<_>>(), reachable);
            for (action, _) in grammar.actions() {
                let distance = words.iter().filter_map(|word| word.iter().position(|&id| id == action)).min();
                prop_assert_eq!(summary.shortest_distance(action), distance.map(|n| n as u64));
            }
        }
    }
}

#[test]
fn rank_saturates_without_changing_order_or_wrapping() {
    use actgram_core::Rank;

    let full = Rank {
        actions: u64::MAX - 1,
        expansions: 0,
    };
    let step = Rank {
        actions: 1,
        expansions: 4,
    };
    let overflow = Rank {
        actions: u64::MAX,
        expansions: u64::MAX,
    };
    assert_eq!(full + step, overflow);
    assert_eq!(full.repeat(2), overflow);
    assert_eq!(full.repeat(0), Rank::STEP);
    let nullable = Rank {
        actions: 0,
        expansions: u64::MAX,
    };
    assert_eq!(nullable.repeat(2), nullable);
    assert_eq!(nullable + Rank::STEP, nullable);
}
