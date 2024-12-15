raugeas
=======

[![crates.io](https://img.shields.io/crates/v/raugeas.svg)](https://crates.io/crates/raugeas) [![CI](https://github.com/normation/raugeas/actions/workflows/ci.yml/badge.svg)](https://github.com/normation/raugeas/actions/workflows/ci.yml)

Rust binding for [Augeas](https://github.com/hercules-team/augeas), a configuration editing tool.

These crates were initially forked from [hercules-team/rust-augeas](https://github.com/hercules-team/rust-augeas).

## Requirements

* Augeas >= 1.13.0
* MSRV: 1.77.0

## Design

This library is a low-level binding to the C API of Augeas, with a few abstractions to make it more idiomatic to use in Rust. It does not aim to provide a high-level API to manipulate configuration files, but rather to provide a safe and idiomatic way to interact with Augeas.
