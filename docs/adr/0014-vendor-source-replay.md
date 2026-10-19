# 0014: Vendored sources replay from published archives and patches

## Context

The root manifest replaces seven crates.io packages with maintained forks.
Upstream revisions and prose modification notes explain their origin, but do
not mechanically reproduce the local sources. Published archives also contain
normalized manifests and publisher packaging which can differ from a git tree.
Some forks retain additional tests and licensed binary fixtures; others omit
published fixtures and packaging files.

## Decision

Every root crates.io patch has one record in `vendor/patches/sources.json` naming
its exact published archive URL and registry SHA-256, upstream revision evidence,
explicit omitted files and ordered binary Git patches. The reconstruction is
the authenticated published archive minus exactly listed omissions plus patches.
Archive metadata distinguishes git VCS evidence from publisher revision assertions.
An assertion does not establish equivalence to a raw upstream git checkout.

`scripts/check-vendor-patches.py` rejects unsafe archive members, authenticates
cached inputs, checks provenance evidence, strictly applies patches and compares
every source file's bytes and executable bits. Only ignored Cargo build output
directories are excluded. Offline replay requires explicit archive inputs and
never silently downloads missing files. A required CI job checks replay and
failure behavior independently of compilation. Vendor changes occupy separate
commits from application code.

## Alternatives rejected

Using prose patch descriptions alone cannot detect unrecorded source changes.
Treating the local vendor tree as an upstream base proves nothing. Using git
revisions without accounting for publisher transformations misstates the exact
inputs. Permissive omission globs could hide unexpected upstream files. Automatic
three-way or whitespace repair would conceal a stale patch.

## Consequences

Historical forks begin with a consolidated binary patch; future changes append
ordered patches. Maintaining patches is additional work, and online replay needs
the pinned archives to remain obtainable. Explicit offline archives support
independent verification. Source replay verifies provenance and bytes; native
behavioral tests and license notice review remain separate requirements.
