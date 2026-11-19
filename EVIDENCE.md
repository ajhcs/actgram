# Directed traversal evidence

Across 1000 seeds per mutation, Eventually detected more failures than Random for 6 mutations, tied for 0, and detected fewer for 4.

## Detections per run

| Mutation | Target | Random | Eventually | Proptest |
|---|---|---:|---:|---:|
| queue-overflow | push_d | 89/1000 | 1000/1000 | 100/1000 |
| queue-drain | peek | 140/1000 | 120/1000 | 156/1000 |
| queue-shrink | shrink_capacity | 100/1000 | 88/1000 | 123/1000 |
| queue-grow | grow_capacity | 85/1000 | 1000/1000 | 94/1000 |
| queue-clear | clear | 54/1000 | 1000/1000 | 50/1000 |
| wal-tail | put_b | 89/1000 | 1000/1000 | 100/1000 |
| wal-ghost | read_a | 140/1000 | 120/1000 | 156/1000 |
| wal-tombstone | delete_a | 100/1000 | 1000/1000 | 123/1000 |
| wal-stale | recover | 85/1000 | 72/1000 | 94/1000 |
| wal-checkpoint | checkpoint | 54/1000 | 1000/1000 | 50/1000 |
| **Total** | | **936/10000** | **6400/10000** | **1046/10000** |

## Median actions through failure and after shrinking

Each cell shows the two medians among detected runs. An em dash means the method found zero failures for that mutation.

| Mutation | Random | Eventually | Proptest |
|---|---:|---:|---:|
| queue-overflow | 19.0 → 6.0 | 6.0 → 6.0 | 18.0 → 6.0 |
| queue-drain | 14.5 → 4.0 | 15.0 → 6.0 | 14.0 → 4.0 |
| queue-shrink | 19.0 → 6.0 | 23.0 → 13.0 | 16.0 → 6.0 |
| queue-grow | 17.0 → 7.0 | 7.0 → 7.0 | 15.0 → 7.0 |
| queue-clear | 16.0 → 5.0 | 5.0 → 5.0 | 18.5 → 5.0 |
| wal-tail | 20.0 → 7.0 | 7.0 → 7.0 | 19.5 → 7.0 |
| wal-ghost | 15.5 → 5.0 | 16.0 → 7.0 | 15.0 → 5.0 |
| wal-tombstone | 21.5 → 8.0 | 8.0 → 8.0 | 19.0 → 8.0 |
| wal-stale | 19.0 → 8.0 | 21.0 → 12.0 | 17.0 → 8.0 |
| wal-checkpoint | 18.0 → 7.0 | 7.0 → 7.0 | 19.5 → 7.0 |

## Reproduction

Run `cargo xtask evidence` from the workspace to repeat the 1,000-seed experiment. Use `--seeds N --out-dir PATH` for a smaller run. The [protocol](evidence/protocol.md) describes the workload, baseline, and limits. The [summary](evidence/summary.csv) and [per-seed data](evidence/seeds.csv) include misses and conditional medians.

Corpus source SHA-256 `f0a2a86683fdc5d719a042c4ddada6c2b4f221154a297d346dad657993c1ce08`.
