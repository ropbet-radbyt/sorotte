# Exact GUI Release Artifact Consumption — 2026-07-30

Status: implemented and proven from a clean committed revision

Implementation revision:
`bb826c4f57e60c3fe425caf20621a1414a38f01e`

Platform: Windows x86_64

## Question

The GUI release workflow built two binaries and produced a ZIP, checksum,
external update manifest, embedded install manifest, and optional symbols. It
then uploaded those files without an independent consumer proving that:

- the uploaded archive was the archive whose checksum and manifests were
  inspected;
- source SHA, channel, version, target, package name, and payload digests agreed
  across both manifests;
- the archive's path and file-type shape was safe and closed;
- the extracted GUI could launch;
- the extracted updater could update an installed copy of itself using the
  exact archive;
- rollback worked in the shipped release binary, not only in synthetic unit
  fixtures;
- the publication job received the same structurally valid bytes as the build
  job.

This slice makes each claim executable. It does not modify updater production
behavior.

## Boundary and threat model

The consumer treats the packaging step, artifact transport, and downloaded
directory as untrusted inputs. Trust begins with the workflow revision and the
expected Git SHA/channel passed separately by the job.

| Boundary | Independent oracle |
|---|---|
| Artifact selection | exactly one versioned Windows x86_64 GUI ZIP |
| Upload directory | exact primary/checksum/update-manifest set, plus an atomic optional symbols/checksum pair |
| Primary bytes | lowercase SHA-256 checksum line bound to the exact filename |
| External manifest | closed schema and exact source/channel/version/target/package/archive-digest agreement |
| ZIP paths | bounded regular files, no encryption, links, special files, absolute paths, traversal, mixed separators, duplicates, or case collisions |
| Embedded manifest | closed schema, exact metadata agreement, exact five-file inventory, and every extracted payload digest |
| Symbols | known non-empty PDB subset, closed inventory, checksum, and safe extraction |
| GUI runtime | fresh profile, saved-connect disabled, injected public/update sources, visible native main window |
| Successful update | updater runs from its installed path, delegates to its authenticated helper, replaces itself, installs every exact package file, commits, and removes transaction artifacts |
| Rollback | a later read-only target makes real `ReplaceFileW` fail after an earlier replacement; the complete original snapshot and clean transaction state are required |
| Publication | downloaded bytes are independently reconsumed before either stable or rolling-dev release publication |

The Windows packager and updater intentionally accept both `\` and `/` in
manifest paths. The consumer normalizes either single separator style to one
canonical path before collision and inventory checks, rejects mixed styles,
and records normalized Windows-style entries in its JSON report. The observed
embedded Lua path is `resources\sorotte_syncplayintf.lua`; its canonical
payload identity is `resources/sorotte_syncplayintf.lua`.

## Implementation

### Independent consumer

`scripts/verify_gui_release_artifact.py`:

1. rejects invalid expected source SHAs and channels before reading artifacts;
2. selects exactly one primary archive and closes the upload-directory
   inventory;
3. verifies checksum syntax, filename, and exact archive bytes;
4. parses both JSON manifests with duplicate-key rejection and closed schemas;
5. cross-binds all shared provenance fields and the archive digest;
6. safely extracts exactly six files, including the install manifest;
7. verifies the five declared payloads byte-for-byte;
8. consumes an optional symbols archive and checksum;
9. launches the exact extracted GUI and requires a visible window;
10. seeds an old install and invokes the exact updater from the installed
    updater path;
11. requires that install to equal the extracted package after self-replacement;
12. invokes the exact helper from the updater's protected bootstrap layout with
    a read-only `README.md` target, then requires the nonzero rollback oracle,
    complete original snapshot, removed journal, and no temp/backup/staging
    files;
13. rehashes the primary archive and external manifest after all work to detect
    in-run substitution;
14. writes an atomic, machine-readable success or failure report.

The consumer shares only the server consumer's adversarial archive/JSON
primitives. GUI identity, schemas, inventory, cross-manifest rules, and runtime
experiments are independent.

### Workflow enforcement

`.github/workflows/sorotte-gui-release.yml` now:

- pins checkout, Rust toolchain, setup-python, upload-artifact, and
  download-artifact to immutable commits;
- records the package channel in the job environment;
- runs the exact consumer after packaging and before package upload;
- always uploads the verification report, while the package upload remains
  success-gated;
- downloads the package into the publication job and reconsumes it with the
  same source/channel contract before either publication branch;
- always uploads a separate publication verification report;
- retains the publication checkout credential because the existing rolling
  dev path intentionally force-updates its scoped release tag.

The publication recheck skips runtime execution: those exact primary bytes
already crossed the runtime boundary before upload, while the publisher
independently repeats selection, checksum, safe extraction, both manifest
checks, payload verification, symbols verification, and source/channel binding.

### Adversarial matrix

`scripts/tests/test_gui_release_artifact.py` adds 33 deterministic tests:

- valid primary and optional-symbol happy paths;
- exact runtime input binding through a mock boundary;
- machine-readable source/channel-bound failure evidence;
- multiple primary archives, checksum mismatch, unexpected upload entries, and
  incomplete symbols pairs;
- invalid source/channel inputs;
- traversal, mixed separators, duplicates, case collisions, missing/extra
  entries, and empty payloads;
- external source, channel, package, digest, version, timestamp, missing-key,
  unknown-key, and duplicate-key drift;
- embedded metadata, digest, inventory, unsafe-path, unknown-key, and
  duplicate-key drift;
- empty, unknown, and checksum-invalid symbols archives;
- immutable action pins;
- package -> consume -> upload ordering;
- download -> reconsume -> publish ordering;
- always-uploaded reports without always-uploaded packages;
- the credential dependency of the rolling dev tag push.

## Experiments and what changed

### 1. Windows separator assumption

The first structural run rejected
`resources\sorotte_syncplayintf.lua`. Inspection showed the external ZIP path
was canonical but the embedded PowerShell-generated install manifest used a
Windows separator. The updater's production parser explicitly splits on both
separator forms and rejects parent, absolute, and drive paths.

Classification: verifier-oracle mismatch, not a product behavior change.

Resolution: the consumer now applies the updater's separator contract before
its stricter canonical collision and inventory checks, and exposes the
normalization in evidence.

### 2. Captured bootstrap stdio

The first installed-updater experiment captured bootstrap stdout/stderr and
hit its 10-second parent timeout. Repeating with bootstrap stdio attached to
the null device allowed immediate delegation and successful update completion.

Classification: harness process-ownership error. The release updater already
launches its detached helper with null stdio; the verifier must not introduce
an inherited pipe lifetime around that process boundary.

Resolution: bootstrap stdio is now explicitly null. The direct rollback helper
remains synchronously captured because it does not delegate again and its exact
nonzero error is the oracle.

### 3. Surfaced product defect: TC-UPDATER-001

The first release-binary rollback experiment authenticated and prepared the
exact package, then changed the already-prepared `README.md` temporary before
replacement. The updater correctly rejected its digest, but emitted:

```text
rollback was incomplete: prepared replacement digest mismatch ...;
recovery journal retained
```

`rollback_journal_entry` authenticates the disposable temporary before it can
remove that temporary or finish journal cleanup. Subsequent recovery repeats
the same failure even when the installed target is still the recognized
original. In the exact-package schedule, rollback restored an earlier changed
file but retained the journal because the corrupt temporary entry failed.

Production behavior was not fixed. The minimized registered characterization
is:

```text
tests::known_defect_tampered_prepared_replacement_blocks_safe_rollback
```

It panics only at:

```text
tampered prepared replacement must not prevent rollback of an unchanged install
```

The registry entry expires on 2026-09-30. The proportional proposed fix is
documented in `docs/TEST_COVERAGE_FINDINGS.md`: preserve strict target/backup
authentication and reparse protection, but treat an uncommitted regular
temporary as disposable scratch during rollback.

### 4. Independent positive rollback proof

The required release gate uses a separate fault that current production
behavior handles correctly. A read-only old `README.md` allows plan creation
and preparation, then makes the second sorted replacement fail after
`LICENSE` has been replaced. The exact release helper returns nonzero with:

```text
all changed files were rolled back
```

The consumer independently verifies every original byte, journal removal, and
absence of all update temp, backup, and staging artifacts.

This positive proof does not turn TC-UPDATER-001 green; the two fault classes
remain separate.

## Clean-commit exact-byte proof

The worktree was clean at:

```text
bb826c4f57e60c3fe425caf20621a1414a38f01e
```

Release build:

```powershell
cargo build --locked --release -p sorotte-gui `
  --bin sorotte-gui --bin sorotte-gui-updater
```

Result: pass in 53.66 seconds.

Package and consume:

```powershell
./scripts/package-gui-release.ps1 `
  -OutputDir target/gui-release-proof-bb826c4 `
  -Channel dev `
  -SkipBuild

python scripts/verify_gui_release_artifact.py `
  --artifacts-dir target/gui-release-proof-bb826c4/artifacts `
  --expected-source-sha bb826c4f57e60c3fe425caf20621a1414a38f01e `
  --expected-channel dev `
  --report target/gui-release-proof-bb826c4/artifact-verification.json
```

Result: verified.

### Uploaded artifact inventory

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `sorotte-gui-0.2.4-windows-x86_64.zip` | 11,129,637 | `bba3dc142e58266bada38de5c4307c302f6351ddb71877af4593b4a1be18d195` |
| primary `.sha256` | 104 | `6c5bedb81cc228334f889a08f7d5d17eb06a8dbf150a2e9b4dec7f79f1ff58da` |
| `sorotte-update-manifest.json` | 390 | `1903e0c485075400284bfab7c451f5782e151a8f2e1b39feaf79ca0d58f586dd` |
| `sorotte-gui-0.2.4-windows-x86_64-symbols.zip` | 3,149,420 | `f23de92c5e2f52efe093a39b0263c95616a95f44e527818c888997da48d8bd6e` |
| symbols `.sha256` | 112 | `8f8076820d10755d4588602abc114d776c955bd77a5853244037e701453eac07` |

### Primary payload inventory

| Canonical path | Bytes | SHA-256 |
|---|---:|---|
| `LICENSE` | 9,226 | `6a84d4f0d3161ee4b5b8287458762ea3b7b0bad4247ecc6457b34cdbab79d619` |
| `README.md` | 5,939 | `1d4d317dfde4f42c47635d6dafe85afa357685d434855e32c4d262e27863b3d9` |
| `resources/sorotte_syncplayintf.lua` | 50,139 | `dc202d33f515038d7d51cf56460d41fa5a94a2160c212e19bbc7644a59101e7a` |
| `sorotte-gui-updater.exe` | 799,232 | `cd98a04dba1f20a36427095688eae4f7ab8cbeafe78fb3b371464666654b6eb5` |
| `sorotte-gui.exe` | 25,182,720 | `af3dc648dd75a3a083b4cbbd43bcfebd9edfb17a4d4ff664378356a6f5758c3f` |

Embedded install-manifest SHA-256:
`df02256fa2fd1ed9390db8e931eb4b5546b314a9ef65ab85059247ae66e66d51`.

The optional symbols archive contained exactly
`sorotte_gui.pdb` and `sorotte_gui_updater.pdb`.

### Runtime evidence

| Experiment | Result | Elapsed |
|---|---|---:|
| Extracted GUI launch | visible window titled `Sorotte GUI`; isolated profile and injected network sources | 669 ms |
| Installed updater | authenticated detached bootstrap, self-replacement, exact package installed, no transaction artifacts | 1,261 ms |
| Faulted rollback | read-only later target, nonzero exit, original snapshot restored, no transaction artifacts | 1,080 ms |
| Total runtime proof | all three passed | 3,077 ms |

Atomic verification report:

- bytes: 3,408;
- SHA-256:
  `a5379d31724ad9ca40fd8cf8b2f1f9fd9900387202f8260557590656b3e1e09b`.

The report binds both runtime log/error digests without embedding
machine-specific temporary paths.

## Validation ledger

Completed before the clean-commit package proof:

| Gate | Result |
|---|---|
| GUI artifact/workflow suite | 33/33 |
| complete Python infrastructure discovery | 354/354 in 13.292 s |
| updater binary unit suite | 21/21, including one registered expected failure |
| Windows updater self-replacement integration | 2/2 |
| known-defect registry | 6 defects / 8 exact characterizations |
| package path boundary suite | pass |
| release publication policy suite | pass |
| `cargo fmt --all --check` | pass |
| actionlint | v1.7.12; GUI workflow pass |
| `cargo test --locked --workspace --all-features` | pass in 188.1 s |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | pass in 1.06 s |

## What this proves

- The exact clean-commit GUI archive and symbols are structurally safe under
  the declared bounded consumer.
- Source SHA and dev channel agree across workflow input, external manifest,
  and embedded manifest.
- Every extracted payload byte agrees with the embedded digest.
- The exact extracted release GUI creates its native main window without
  relying on a real server, public DNS, or a player.
- The exact extracted release updater can bootstrap from its installed path,
  replace that updater, and install all exact package bytes transactionally.
- A real Windows replacement failure after earlier mutation restores the
  complete old install and cleans transaction state.
- The package cannot reach artifact upload or stable/dev publication if its
  corresponding consumer fails.
- A distinct tampered-temporary recovery defect is preserved rather than
  hidden by the positive rollback proof.

## Non-claims

This is not proof of:

- Authenticode, Sigstore, SBOM, or dependency provenance;
- GitHub's final public release asset digest matching the uploaded Actions
  artifact after publication;
- installer behavior under elevation or protected system installation roots;
- power-loss durability, kernel cache flush guarantees, disk exhaustion, or
  malware resistance;
- every native GUI interaction or accessibility contract (those remain in the
  native semantic/smoke layers);
- the open TC-UPDATER-001 tampered-temporary schedule succeeding.

## Next boundary

The next high-leverage release slice is the server container: build once,
consume the exact image by digest, assert non-root filesystem/network policy,
run the protocol Hello contract through the container, inspect layers and
declared ports/entrypoint, and compare the pushed registry digest with the
tested local content. Post-publication GUI/server asset digest comparison and
SBOM/signature policy can then share that provenance ledger.
