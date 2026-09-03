# ADR-0001: One Rust crate

*Status: **ADOPTED** (sealed, Phase A)*

## Context

VOLE must remain auditable and self-contained. Micro-crates would fragment
materialization semantics and inflate review surface without measured benefit.

## Decision

VOLE is shipped as **one Rust crate** (`Cargo.toml`, `src/lib.rs`), native
Rust, `edition 2021`, `#![forbid(unsafe_code)]`. DSFB and EntropyFS are
**separate repositories** and are never normative dependencies.

## Consequences

- One unit of versioning, one format + materializer boundary.
- Native Rust only: no FFmpeg/libavcodec/x264/… wrappers; external codecs are
  only benchmark baselines.
- Simplicity over modular novelty: no empty "architecturally-sophisticated"
  placeholder directories.
