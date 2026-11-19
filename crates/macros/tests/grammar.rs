use actgram_core::GrammarError;
use actgram_macros::grammar;

#[test]
fn macro_supports_recursive_nullable_rules_and_unbounded_repeats() {
    let grammar = grammar! {
        start main;
        main = seq(repeat(@trail, 0, *), check);
        trail = alt(1 => seq(), 2 => seq(r#type, @trail), 0 => disabled);
    }
    .unwrap();
    let names: Vec<_> = grammar.analysis()[grammar.root()]
        .first()
        .map(|id| grammar.action_name(id).unwrap())
        .collect();
    assert_eq!(names, ["check", "type"]);
}

#[test]
fn macro_uses_the_builder_validation() {
    let error = grammar! { start main; main = repeat(read, 2, 1); }.unwrap_err();
    assert!(matches!(
        error,
        GrammarError::BadRepeatBounds { min: 2, max: 1, .. }
    ));
    let error = grammar! { start unknown; }.unwrap_err();
    assert!(matches!(error, GrammarError::UndefinedSymbol { name, .. } if name == "unknown"));
}
