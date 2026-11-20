# Temporary Rust dependency backports

This workspace keeps Rust 1.89 and leaves its dependency audit policy enabled.
It carries source from these crates.io archives (SHA-256 checked against the
original workspace lockfile):

| Crate | Archive SHA-256 |
| --- | --- |
| [rstar 0.12.2](https://crates.io/crates/rstar/0.12.2) | `421400d13ccfd26dfa5858199c30a5d76f9c54e0dba7575273025b43c5175dbb` |
| [geo-types 0.7.20](https://crates.io/crates/geo-types/0.7.20) | `777d18aa0f12f8b285331cd867133ee14422b3f023f6d388034c47d43e28786a` |

Both retain their upstream MIT/Apache-2.0 license texts. For rstar these were
retrieved from the [upstream v0.12.2 tag](https://github.com/georust/rstar/tree/v0.12.2).
Crate Rust sources are unmodified.

- `rstar`: allow `heapless >=0.9.4,<0.10`, following [upstream's dependency
  bump](https://github.com/georust/rstar/pull/249). That is the fixed series for
  RUSTSEC-2026-0574 through RUSTSEC-2026-0582. The backport raises rstar's
  declared Rust minimum to 1.88 to match heapless.
- `geo-types`: retain only the active `rstar_0_12` feature and its alias.
  The unused optional integrations brought older heapless versions into the
  lockfile even though the binaries did not build them. Only its manifest is
  patched; unrelated features are preserved.

The server/CLI builds use the workspace patch, including in Docker. The
separate fuzz workspace mirrors it explicitly. This is not a downstream
crates.io patch.

See [upstream tracking issue #273](https://github.com/georust/rstar/issues/273).
Remove the backports, Docker copies, release license copy, and fuzz overrides
together once published
compatible dependencies resolve with Rust 1.89, old heapless versions are gone
from the regenerated lockfiles, and the existing audit and CI pass.
