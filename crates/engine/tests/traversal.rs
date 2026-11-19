use actgram_core::{Builder, Grammar};
use actgram_engine::{Choices, Engine, EngineError, Eventually, Next, Random};
use proptest::prelude::*;

fn figure_24() -> Grammar {
    // US7290193B2, fig. 24: S1 = a (b (S2 | S3) g)*.
    let mut b = Builder::new();
    let a = b.action("a");
    let b_action = b.action("b");
    let c = b.action("c");
    let d = b.action("d");
    let e = b.action("e");
    let f = b.action("f");
    let g = b.action("g");
    let ce = b.seq([c, e]);
    let df = b.seq([d, f]);
    let dfs = b.repeat(df, 1, Some(3));
    b.rule("S2", ce).unwrap();
    b.rule("S3", dfs).unwrap();
    let s2 = b.symbol("S2");
    let s3 = b.symbol("S3");
    let alternatives = b.alt([(1, s2), (1, s3)]);
    let block = b.seq([b_action, alternatives, g]);
    let blocks = b.repeat(block, 0, None);
    let sequence = b.seq([a, blocks]);
    b.rule("S1", sequence).unwrap();
    let start = b.symbol("S1");
    b.build(start).unwrap()
}

/// A recognizer that sees only the emitted word and the figure's production rules.
fn recognizes_figure_24(mut word: &str) -> bool {
    let Some(rest) = word.strip_prefix('a') else {
        return false;
    };
    word = rest;
    while !word.is_empty() {
        let Some(rest) = word.strip_prefix('b') else {
            return false;
        };
        word = rest;
        if let Some(rest) = word.strip_prefix("ce") {
            word = rest;
        } else {
            let mut count = 0;
            while let Some(rest) = word.strip_prefix("df") {
                count += 1;
                word = rest;
            }
            if !(1..=3).contains(&count) {
                return false;
            }
        }
        let Some(rest) = word.strip_prefix('g') else {
            return false;
        };
        word = rest;
    }
    true
}

#[test]
fn figure_24_traversal_accepts_and_replays() {
    let grammar = figure_24();
    let generate = |choices: &mut Choices<'_>| {
        let mut engine = Engine::new(&grammar);
        let mut word = String::new();
        while let Some(action) = engine.advance(choices).unwrap().action {
            word.push_str(grammar.action_name(action).unwrap());
        }
        word
    };
    for seed in 0..512 {
        let mut choices = Choices::seeded(seed);
        let word = generate(&mut choices);
        assert!(recognizes_figure_24(&word), "{word}");
        let mut same_seed = Choices::seeded(seed);
        let mut tape = Choices::replay(choices.observed());
        assert_eq!(word.as_bytes(), generate(&mut same_seed).as_bytes());
        assert_eq!(word, generate(&mut tape));
        assert_eq!(choices.observed(), tape.observed());
    }
}

#[test]
fn next_skips_nullable_recursion_and_restores_previous_strategy() {
    let mut b = Builder::new();
    let cycle = b.symbol("zero");
    let epsilon = b.seq([]);
    let zero = b.alt([(u32::MAX, cycle), (1, epsilon)]);
    b.rule("zero", zero).unwrap();
    let skips = b.repeat(zero, 1, None);
    let a = b.action("a");
    let target = b.action("target");
    let root = b.seq([skips, a, target]);
    let grammar = b.build(root).unwrap();
    let a = grammar.action("a").unwrap();
    let target = grammar.action("target").unwrap();
    let mut choices = Choices::replay(&[]);
    let mut engine = Engine::new(&grammar);

    engine.push_strategy(Eventually(target));
    engine.push_strategy(Next(target));
    assert_eq!(
        engine.advance(&mut choices),
        Err(EngineError::Unreachable(Next(target)))
    );
    assert!(engine.pop_strategy());
    engine.push_strategy(Next(a));
    assert_eq!(engine.advance(&mut choices).unwrap().action, Some(a));
    assert_eq!(engine.strategy(), Eventually(target));
    assert_eq!(engine.advance(&mut choices).unwrap().action, Some(target));
    assert_eq!(engine.strategy(), Random);
    assert!(!engine.pop_strategy());
}

#[test]
fn limits_and_strict_tape_errors_are_observable() {
    let mut b = Builder::new();
    let rec = b.symbol("R");
    let a = b.action("a");
    let r = b.alt([(1, rec), (1, a)]);
    b.rule("R", r).unwrap();
    let grammar = b.build(rec).unwrap();
    let mut engine = Engine::new(&grammar);
    engine.set_expansion_limit(5);
    assert_eq!(
        engine.advance(&mut Choices::replay(&[0, 0, 0])),
        Err(EngineError::ExpansionLimit(5))
    );
    let mut engine = Engine::new(&grammar);
    assert!(matches!(
        engine.advance(&mut Choices::replay(&[])),
        Err(EngineError::Tape(_))
    ));
    let mut engine = Engine::new(&grammar);
    assert!(matches!(
        engine.advance(&mut Choices::replay(&[2])),
        Err(EngineError::Tape(_))
    ));
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, failure_persistence: None, ..ProptestConfig::default() })]
    #[test]
    fn directed_goals_match_analysis(
        seed in any::<u64>(),
        flags in prop::array::uniform6(any::<bool>()),
        lower in 0u32..=3,
    ) {
        let mut b = Builder::new();
        let a = b.action("a");
        let finish = b.action("finish");
        let hidden = b.action("hidden");
        let eps = b.seq([]);
        let back = b.symbol("choice");
        let repeated = b.repeat(if flags[0] { a } else { eps }, lower, flags[1].then_some(3));
        let recursion = b.seq([if flags[2] { repeated } else { eps }, back]);
        let leaf = b.alt([(3, if flags[3] { a } else { eps }), (u32::from(flags[4]), hidden)]);
        let branch = b.alt([(100, recursion), (1, leaf)]);
        b.rule("choice", branch).unwrap();
        let root = b.seq([repeated, back, if flags[5] { finish } else { eps }]);
        let grammar = b.build(root).unwrap();

        for (target, _) in grammar.actions() {
            let distance = grammar.analysis()[root].shortest_distance(target);
            let mut engine = Engine::new(&grammar);
            engine.push_strategy(Next(target));
            let mut choices = Choices::seeded(seed);
            let immediate = engine.advance(&mut choices);
            if distance == Some(0) {
                prop_assert_eq!(immediate.unwrap().action, Some(target));
            } else {
                prop_assert_eq!(immediate, Err(EngineError::Unreachable(Next(target))));
            }

            let mut engine = Engine::new(&grammar);
            engine.push_strategy(Eventually(target));
            if let Some(distance) = distance {
                for index in 0..=distance {
                    let action = engine.advance(&mut choices).unwrap().action.unwrap();
                    prop_assert_eq!(action == target, index == distance);
                }
                prop_assert_eq!(engine.strategy(), Random);
            } else {
                prop_assert_eq!(engine.advance(&mut choices), Err(EngineError::Unreachable(Eventually(target))));
            }
        }
    }
}

#[test]
fn choice_tape_skips_forced_draws_and_adapts_only_in_edit_mode() {
    use std::num::NonZeroU64;

    let one = NonZeroU64::MIN;
    let three = NonZeroU64::new(3).unwrap();
    let mut strict = Choices::replay(&[7]);
    assert_eq!(strict.draw(one).unwrap(), 0);
    assert!(strict.observed().is_empty());
    assert!(strict.draw(three).is_err());
    assert!(strict.observed().is_empty());

    let mut edited = Choices::edited(&[7]);
    assert_eq!(edited.draw(three).unwrap(), 1);
    assert_eq!(edited.draw(three).unwrap(), 0);
    assert_eq!(edited.into_tape(), [1, 0]);

    let wide = NonZeroU64::new((1 << 63) + 1).unwrap();
    let mut seeded = Choices::seeded(17);
    let expected: Vec<_> = (0..16).map(|_| seeded.draw(wide).unwrap()).collect();
    let mut replay = Choices::replay(seeded.observed());
    assert_eq!(
        expected,
        (0..16)
            .map(|_| replay.draw(wide).unwrap())
            .collect::<Vec<_>>()
    );
    assert!(replay.draw(wide).is_err());
}
