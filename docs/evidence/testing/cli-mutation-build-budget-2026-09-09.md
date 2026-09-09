# CLI mutation compilation budget

The open PR campaign on `6ffa92abec234cf75a52efb042e02efe9675f63b`
exceeded the CLI framing shard's 120-second compilation limit. The failed
mutant never reached its test phase. Its raw outcome records a `Build` timeout
at 120.025 seconds while Cargo was building the CLI package and integration
binaries with two concurrent mutation workers.

The [original job](https://github.com/ropbet-radbyt/sorotte/actions/runs/34343062456/job/102438366772)
caught 24 mutants, reported two reviewed unviable mutants, and timed out while
compiling one. The baseline compilation took 91.734 seconds. The retained
`sorotte-mutation-chunk-cli-framing--1-of-2-1` artifact contains the complete
report and `results/mutants.out/outcomes.json`.

A single [diagnostic retry](https://github.com/ropbet-radbyt/sorotte/actions/runs/34343062456/job/102452775590)
compiled the same mutant in 108.703 seconds and caught it through a failing
test in 27.742 seconds. It caught all 25 viable mutants, with the same two
reviewed unviable mutants and no timeouts. The complete result is retained in
`sorotte-mutation-chunk-cli-framing--1-of-2-2`. This demonstrates limited
compilation headroom; it does not establish a stable upper runtime bound.

The required aggregate correctly refused to replace the completed failure with
an unchanged retry. That restriction remains intact. The failed campaign is
not qualification evidence for a merge or release.

The correction gives this package-wide compilation phase 240 seconds, matching
the existing budget used by several other substantial shards. The test phase
still has 60 seconds. Mutation inventory, package-wide test scope, worker count,
100% viable kill requirement, and zero allowed survivors/timeouts are unchanged.
The corrected policy requires a new campaign on its committed PR revision;
neither earlier attempt can qualify that revision. Actual timings remain in
each raw outcome and the aggregate campaign receipt.
