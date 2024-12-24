# Changelog

## 0.3.0 (2024-12-24)

- Support non-UTF-8 strings ([e7db])
- Add FromStr impl for Position ([00e2])
- Add clear/clearm methods ([e60b])
- Add a touch method ([5af0])
- Use Cow for str escaping ([9526])

[e7db]: https://github.com/Normation/raugeas/commit/ca2790e6ecc127a9062324f60227a907cc56e7db
[00e2]: https://github.com/Normation/raugeas/commit/95b3c2b29d56c95049ff04868ae631677d4f00e2
[e60b]: https://github.com/Normation/raugeas/commit/2cbd2729369f438236f4ec01214e80fcbbd2e60b
[5af0]: https://github.com/Normation/raugeas/commit/a9e3bc7e13eecabf5494367d440aaa9ee2df5af0
[9526]: https://github.com/Normation/raugeas/commit/c1af5cfaeea80d4a5244c5b4d3cd82938b0d9526

## 0.2.2 (2024-12-15)

- Add a helper enum and method for save modes ([62f4])
- Implement std::Error for augeas::Error ([6420])

[62f4]: https://github.com/Normation/raugeas/commit/62f4a58242ce2950a48975855cc5e74c3b66da0e
[6420]: https://github.com/Normation/raugeas/commit/6420ee041245c3c33b25852d265196f40a6e081f

## 0.2.1 (2024-12-15)

- Add support for `aug_print` and `aug_srun` ([b654], [dc57])

[b654]: https://github.com/Normation/raugeas/commit/b654139242129d366c0874ec3379f8c852f7131a
[dc57]: https://github.com/Normation/raugeas/commit/dc576be9f438bda4c06598f0af081dac25c9a662

## 0.2.0 (2024-11-28)

- Merge "Add bindings up to Augeas 1.11.0" PR [#10] from rust-augeas ([2394])
- Merge "Refactor error handling" PR [#11] from rust-augeas ([8f9a])
- Code update and cleanup ([67bc])
    - MSRV is bumped to 1.77
    - Switch to edition 2021
    - Add GitHub Actions for CI
- Update to augeas 1.13.0 API ([de37])


[#10]: https://github.com/hercules-team/rust-augeas/pull/10
[#11]: https://github.com/hercules-team/rust-augeas/pull/11

[2394]: https://github.com/Normation/raugeas/commit/2394d3ec362186eecd28fb839a410447b3dba439
[8f9a]: https://github.com/Normation/raugeas/commit/8f9a41e3f416a4bd8ac3badfb1b08d156d6a0c2f
[67bc]: https://github.com/Normation/raugeas/commit/67bc7d0747fb31ca7ca6135f8e4e6e54bc0e8763
[de37]: https://github.com/Normation/raugeas/commit/de37a83e3725e7231259befc39e3494ea79a0b26