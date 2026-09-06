# Reviewed Python verification inputs

Install the requirement file for the task: `requirements/ci-policy.txt` for YAML
policy tests, `requirements/legacy-python-interop.txt` for the live Python
reference, and `requirements/dependency-audit.txt` for the dependency auditor.
Each selects its own top-level packages and references the same reviewed
`verification-constraints.txt` beside it. Constraints limit selected versions;
they do not install the auditor in the interoperability environment. Dependency
environment markers still decide whether conditional packages are needed.

The 2026-09-06 review resolved 44 distinct package versions. Actual pip dry runs
covered all three environments on native Windows x86-64 and WSL Ubuntu x86-64,
targeting CPython 3.11, 3.12, and 3.13. All 18 initial resolutions and all 18
constrained replays succeeded, with identical package selections in each pair.
An independent report review also checked every active dependency, specifier,
environment marker, requested extra, and selected wheel hash in the closure.
Reports, resolver commands, OS/interpreter identity, selected wheel URLs and
SHA-256 hashes are retained locally under
`target/verification/python-resolution/`. No global packages were installed.

The native resolvers were Windows Python 3.13.5/pip 25.1.1 and Ubuntu Python
3.12.3/pip 24.0. The resolver's Python markers and wheel ABI were explicitly
projected for each target version; OS markers remained native. These are
dependency resolution checks, not evidence of executing the suite under six
separate interpreters. The report context records this distinction. Merely
passing pip `--python-version` does not override every environment marker.

`coverage/verification-tools.toml` records the reviewed constraint digest as
UTF-8 text normalized to LF. Static preflight checks this digest, exact local
constraint directives, top-level inventories, and duplicated Rust/tool/legacy
pin literals using TOML and Python AST parsing. It does not import wrappers,
compile binaries, or contact the network. Run it with:

```console
python scripts/verify.py preflight --phase static --output target/verification/preflight.json
```

Interop also checks the constraint input against the reviewed digest before
probing the interpreter. Its historical v1 report shape remains compatible;
the full source identity and release qualification input closure bind the
constraint file's actual bytes. The canonical review digest does not replace
that physical-byte identity.

To update Python inputs, resolve each environment independently in isolated
temporary environments for the supported OS/Python matrix, retain pip's
`--dry-run --ignore-installed --only-binary=:all: --report` output, review the
complete package closure and selected wheel hashes, update constraints and
the manifest digest, and replay the matrix with constraints. Preserve marker
conditions and prove that requirements for one environment have not pulled
another environment's tools into it. Resolution hashes are review evidence;
the constraints file pins versions and does not claim pip hash enforcement.
