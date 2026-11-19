# Evidence protocol

The experiment asks whether choosing the shortest grammar path to a named
action detects a mutation in more seeded runs than weighted random traversal.
The comparison uses a bounded FIFO and a simulated write-ahead log with
independent reference state. The commands, weights, mutations and assigned
targets are fixed in [the corpus](../examples/src/corpus.rs). Each nonempty
workload begins by resetting the system. The observation commands compare
return values or visible state with the reference.

Each bug runs separately with only its mutation active. Each method gets
1,000 fresh runs with seeds 0 through 999. A run offers eight workload slots
and stops at the first detected mismatch. A slot selects a weighted workload,
including a skip with no commands. All methods use the same operations,
reference checks, weights, and slot budget. The directed method begins with
one Eventually goal, returns to weighted random on hitting it, and continues
the selected workload. Some goals identify a rare workload. Others, such
as a read or recovery, also occur on short paths with no useful setup.

The proptest-state-machine baseline uses Sequential from version 0.9.0 with
proptest 1.12.0 and its ChaCha generator. One state-machine transition
represents a reset-delimited workload or skip. Its weighted sampler matches
the random grammar's workload distribution. Its shrinker can delete or
replace whole transitions. The experiment's Actgram grammar likewise cannot
change the commands within a fixed workload. The baseline calls the same
operations and checks directly. Timing and scheduler behavior are outside
this experiment.

The primary measure is detections per 1,000 runs, compared per bug.
The report also gives the median number of started actions through the
failure and the median number of actions in the reduced reproductions,
conditional on a detection. An observation that fails and a reset both
count as actions. Each shrinker is allowed 512 evaluations after its
initial check. Driver errors fail generation of the report. The clean
systems and every mutation's trigger are tested independently.

This measures targeting with a supplied goal under two fixed workloads.
Common actions may take a short path that skips the required state. The
target choices encode workload knowledge, and the results cover synthetic
mutations in two small systems. Keep all outcomes in the report.
