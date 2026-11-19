#![no_main]

use actgram_core::{Builder, Node, NodeId};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let mut b = Builder::new();
    let empty = b.seq([]);
    let read = b.action("read");
    let write = b.action("write");
    let mut nodes = vec![empty, read, write];
    for chunk in input.chunks_exact(5).take(64) {
        let reference = |byte: u8| {
            if byte == 255 {
                return NodeId::from_index(999);
            }
            nodes[usize::from(byte) % nodes.len()]
        };
        let a = reference(chunk[1]);
        let c = reference(chunk[2]);
        let min = u32::from(chunk[3] % 5);
        let max = u32::from(chunk[4] % 7);
        let node = match chunk[0] % 8 {
            0 => b.action(format!("a{}", chunk[1] % 8)),
            1 => b.symbol("S"),
            2 => b.symbol("T"),
            3 => b.seq([a, c]),
            4 => b.alt([(min, a), (max, c)]),
            5 => b.repeat(a, min, Some(max)),
            6 => b.repeat(a, min, None),
            _ => b.seq([]),
        };
        nodes.push(node);
    }
    let select = |byte: u8| nodes[usize::from(byte) % nodes.len()];
    let a = input.first().copied().unwrap_or(0);
    let c = input.get(1).copied().unwrap_or(1);
    if a % 8 != 0 {
        b.rule("S", select(a)).unwrap();
    }
    if c % 8 != 0 {
        b.rule("T", select(c)).unwrap();
    }
    let root = if c == 255 {
        NodeId::from_index(999)
    } else {
        *nodes.last().unwrap()
    };
    if let Ok(grammar) = b.build(root) {
        for (id, node) in grammar.nodes() {
            let summary = &grammar.analysis()[id];
            let minimum = summary.minimum_actions().expect("productive node");
            assert_eq!(summary.nullable(), minimum == 0);
            for action in summary.first() {
                assert_eq!(summary.shortest_distance(action), Some(0));
            }
            if let Node::Action(action) = node {
                assert_eq!(minimum, 1);
                assert_eq!(summary.first().collect::<Vec<_>>(), [*action]);
            }
            if let Node::Symbol(name) = node {
                assert!(grammar.rule(name).is_some());
            }
        }
    }
});
