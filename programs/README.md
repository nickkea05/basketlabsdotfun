# programs

Anchor workspace for the on-chain side of basketlabs.fun. The workspace root is the
repository root (`Anchor.toml`, `Cargo.toml`, `rust-toolchain.toml` live there);
programs live here.

- `basket/` — factory program. One deployment; each launch creates a basket account,
  a share mint and a program-owned vault. Instruction arguments mirror
  `buildLaunchParams()` in `packages/core` (`CreateBasketArgs`), so the web app and
  the program share one schema. Currently an empty skeleton: modules, constants and
  errors only, no instructions.

The Anchor client and IDL will be published from `packages/solana` once the program
has instructions. `@anchor-lang/core` (the renamed `@coral-xyz/anchor`) is already a
dependency there.

## Toolchain

Pinned so the whole chain agrees with itself. Change one and re-check the others.

| Piece                | Version | Where it is pinned                                |
| -------------------- | ------- | ------------------------------------------------- |
| Anchor CLI / crates  | 1.2.0   | `Anchor.toml` `[toolchain]`, `Cargo.toml`         |
| Solana CLI (Agave)   | 4.3.0   | Installed on the machine (see below), not in toml |
| platform-tools (SBF) | v1.57   | Anchor 1.2.0 default = what Solana 4.3.0 bundles  |
| Host Rust            | 1.96.1  | `rust-toolchain.toml` (matches Agave 4.2)         |
| LiteSVM              | 0.16.0  | `programs/basket/Cargo.toml` dev-deps             |
| `@anchor-lang/core`  | 1.2.0   | `packages/solana/package.json`                    |

How they fit: Anchor 1.2.0 asks `cargo build-sbf` for platform-tools v1.57. Solana
4.3.0 ships v1.57 built in, so the build never has to fetch a second toolchain
(4.1.2 ships v1.54 and its `cargo-build-sbf` cannot fetch v1.57 on Windows). LiteSVM
0.16 is the first release that loads the SBPF v3 programs Anchor 1.x emits, and it is
built on Agave 4.2 runtime crates, which need Rust 1.96. Agave 4.3's own toolchain is
newer than 1.96 but the crates LiteSVM pulls are 4.2.x.

Install (any OS):

```
rustup toolchain install 1.96.1
cargo install avm --git https://github.com/otter-sec/anchor --tag v1.2.0 --locked
avm install 1.2.0 && avm use 1.2.0
# Solana CLI 4.3.0: https://release.anza.xyz  (agave-install init v4.3.0)
```

`[toolchain] solana_version` is deliberately absent from `Anchor.toml`. When set,
Anchor swaps the active CLI with `agave-install` before every command and swaps it
back after; on Windows `agave-install` needs admin, so the swap fails and leaves no
active release at all.

## Commands

```
anchor build        # SBF build + IDL to target/idl, TS types to target/types
anchor test         # anchor build, then `cargo test` (LiteSVM, no validator needed)
npm run anchor:test # same, from the repo root
anchor keys list    # program id from target/deploy/basket-keypair.json
```

Tests are Rust + LiteSVM under `programs/basket/tests/`, one file per instruction,
written before the handler. `anchor test` does not start a validator
(`skip_local_validator = true`). For a real local cluster use
`anchor localnet --validator legacy`; the default backend is Surfpool, which is not
installed here.

## Program id

`declare_id!` in `lib.rs` matches `target/deploy/basket-keypair.json`, which is
gitignored and exists only on the machine that generated it. It is a dev id. The
mainnet id comes from a keypair kept outside the repository; `anchor keys sync`
rewrites `declare_id!` and `Anchor.toml` when it changes.

## Windows notes

Anchor's docs say WSL; native Windows works for `anchor build` and `anchor test` with
two caveats found while setting this up:

- `agave-install` needs admin to create the `active_release` symlink. Without it the
  release downloads but the link is missing (and an existing link is removed). A
  directory junction works and needs no admin:
  `New-Item -ItemType Junction -Path ~\.local\share\solana\install\active_release -Target ~\.local\share\solana\install\releases\4.3.0\solana-release`.
  Remove a stale one with `cmd /c rmdir <path>` (removes the link, not the target).
- `cargo-build-sbf` from Solana 4.1.x panics on Windows whenever the requested
  platform-tools version is not the bundled one (`toolchain.rs`, `Option::unwrap` on
  a `rustc` path probe with no `.exe`). Use a Solana release whose bundled version
  matches what Anchor asks for; 4.3.0 + v1.57 does.
- A partial platform-tools download leaves a half-populated `~/.cache/solana/vX.YY`
  that the next run trips over. Delete the directory and run again.
- Antivirus that holds executed binaries open (McAfee does) makes the relink after a
  source change fail with `LNK1104: cannot open file ...\target\debug\deps\*.exe`.
  Exclude `basketfun\target` from on-access scanning, or build from WSL.
