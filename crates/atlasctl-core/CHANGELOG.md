# Changelog

## [0.4.0](https://github.com/Avarok-Cybersecurity/atlas-recipes/compare/v0.3.0...v0.4.0) (2026-08-29)


### Features

* **recipes:** a recipe for Qwen3.8-Flash-Next ([#120](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/120)) ([3d2db11](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/3d2db1180d7d29732554fa4a48ca2a17e8a3feb7))
* **run:** answer an unknown `-o` key instead of only refusing it ([#103](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/103)) ([0012bb1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/0012bb1df224bbbd665978e65982440aed5a01eb))
* **run:** say the weights are missing before pulling an image for nothing ([#100](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/100)) ([491e6ed](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/491e6ed03c29d6bc106dcc7692c4c0f09ceed280))
* **run:** warn before a GPU utilisation that has frozen a machine ([#72](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/72)) ([68493e1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/68493e1cfb1014b34975df4ac9c730271c49280b))
* **telemetry:** measure ISL and OSL, so the control page stops promising them ([#65](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/65)) ([2a2c7f7](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2a2c7f7d666d01533588afba16adb5ef8ddb9549))
* **windows:** native Windows support — binary, installer, and supervised agent ([#114](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/114)) ([e8b304a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e8b304aa07c190f017278657c1719bc533b2311a))


### Bug Fixes

* a destructive git escape the path guard could not see, and three more ([#117](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/117)) ([f2fcabe](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/f2fcabe83534d1cf9aa673a52db55196e8caf431))
* **agent:** four ways the installed agent misbehaved on a user's machine ([#119](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/119)) ([e5fc285](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e5fc285d4a08056e4a8f6df2a243d97bb0c60e9d))
* **cache:** a directory is not a downloaded model ([#150](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/150)) ([b83b327](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/b83b32744c7eb59ddc09f9e32f4f272c615103db))
* **cache:** a nested weight file is still a weight file ([#154](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/154)) ([1ff0854](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/1ff085413a8002ad7925859e1427b0546781233f))
* **docker:** mark what the renderer wrote, do not guess it from the text ([#90](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/90)) ([76e6043](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/76e60430821755be5b37a7212a5867eda39a5d42))
* **nearest:** suggest the name the operator half-typed ([#166](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/166)) ([d04d386](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d04d3868515ce5c03170dc3ef890a4809cdc1808))
* **recipe:** a recipe may not read the agent's secrets or its egress policy ([#91](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/91)) ([9fe4b3a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/9fe4b3a63097163f683a8cb18211c2f8a483fe77))
* **recipe:** refuse a field that would be read as an option ([#93](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/93)) ([6487c66](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/6487c669568d279c0e086ee78bdddd57effec4f1))
* **recipe:** suggest the recipe you meant, not the three that sort first ([#96](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/96)) ([e88874d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e88874d3b2ffca70537ace311624b8f2f9d54508))
* **registry:** a scope should not cost the operator the suggestion ([#111](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/111)) ([5b6eb5e](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/5b6eb5e7d670d19ad1e145212f3092ea9c4c8644))
* **registry:** a scoped ref cannot walk out of the recipe cache ([#92](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/92)) ([46eccef](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/46eccef8c07965e03544b29c9da057efbc2f6d30))
* **run:** guard the endpoint port against the engine's own default ([#68](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/68)) ([9189d8c](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/9189d8c28eb9712f1aa91ae3db717764891c4f3a))
* **run:** the CLI never checked a `-o` value against its declared bound ([#102](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/102)) ([c3ae320](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/c3ae320362b3833ea1c6ae433c6825450d34b443))
* **secretfile:** canonicalising every write raced Windows file replacement ([#132](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/132)) ([efe1325](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/efe132585c2756a28d04f3e1a3126b1db60ece31))
* **settings:** `port` is the TCP domain, not the IANA registered range ([#105](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/105)) ([d971dd1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d971dd1e8da22cec0e83d00640dd0cab7a911527))
* six audit findings, including two regressions from tonight ([#104](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/104)) ([7e3386f](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/7e3386fa62de3c866292fa87dbebc132a68ca135))

## [0.3.0](https://github.com/Avarok-Cybersecurity/atlas-recipes/compare/v0.2.0...v0.3.0) (2026-08-29)


### Features

* **recipes:** a recipe for Qwen3.8-Flash-Next ([#120](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/120)) ([3d2db11](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/3d2db1180d7d29732554fa4a48ca2a17e8a3feb7))
* **run:** answer an unknown `-o` key instead of only refusing it ([#103](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/103)) ([0012bb1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/0012bb1df224bbbd665978e65982440aed5a01eb))
* **run:** say the weights are missing before pulling an image for nothing ([#100](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/100)) ([491e6ed](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/491e6ed03c29d6bc106dcc7692c4c0f09ceed280))
* **run:** warn before a GPU utilisation that has frozen a machine ([#72](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/72)) ([68493e1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/68493e1cfb1014b34975df4ac9c730271c49280b))
* **telemetry:** measure ISL and OSL, so the control page stops promising them ([#65](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/65)) ([2a2c7f7](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2a2c7f7d666d01533588afba16adb5ef8ddb9549))
* **windows:** native Windows support — binary, installer, and supervised agent ([#114](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/114)) ([e8b304a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e8b304aa07c190f017278657c1719bc533b2311a))


### Bug Fixes

* a destructive git escape the path guard could not see, and three more ([#117](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/117)) ([f2fcabe](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/f2fcabe83534d1cf9aa673a52db55196e8caf431))
* **agent:** four ways the installed agent misbehaved on a user's machine ([#119](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/119)) ([e5fc285](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e5fc285d4a08056e4a8f6df2a243d97bb0c60e9d))
* **cache:** a directory is not a downloaded model ([#150](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/150)) ([b83b327](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/b83b32744c7eb59ddc09f9e32f4f272c615103db))
* **cache:** a nested weight file is still a weight file ([#154](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/154)) ([1ff0854](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/1ff085413a8002ad7925859e1427b0546781233f))
* **docker:** mark what the renderer wrote, do not guess it from the text ([#90](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/90)) ([76e6043](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/76e60430821755be5b37a7212a5867eda39a5d42))
* **flags:** claim the nine recipe settings that never reached the engine ([#61](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/61)) ([d7f9ea2](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d7f9ea26692d999d499bacf9835b181f9ac33fa4))
* **recipe:** a recipe may not read the agent's secrets or its egress policy ([#91](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/91)) ([9fe4b3a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/9fe4b3a63097163f683a8cb18211c2f8a483fe77))
* **recipe:** refuse a field that would be read as an option ([#93](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/93)) ([6487c66](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/6487c669568d279c0e086ee78bdddd57effec4f1))
* **recipe:** suggest the recipe you meant, not the three that sort first ([#96](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/96)) ([e88874d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e88874d3b2ffca70537ace311624b8f2f9d54508))
* **registry:** a scope should not cost the operator the suggestion ([#111](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/111)) ([5b6eb5e](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/5b6eb5e7d670d19ad1e145212f3092ea9c4c8644))
* **registry:** a scoped ref cannot walk out of the recipe cache ([#92](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/92)) ([46eccef](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/46eccef8c07965e03544b29c9da057efbc2f6d30))
* **run:** guard the endpoint port against the engine's own default ([#68](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/68)) ([9189d8c](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/9189d8c28eb9712f1aa91ae3db717764891c4f3a))
* **run:** the CLI never checked a `-o` value against its declared bound ([#102](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/102)) ([c3ae320](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/c3ae320362b3833ea1c6ae433c6825450d34b443))
* **secretfile:** canonicalising every write raced Windows file replacement ([#132](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/132)) ([efe1325](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/efe132585c2756a28d04f3e1a3126b1db60ece31))
* **settings:** `port` is the TCP domain, not the IANA registered range ([#105](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/105)) ([d971dd1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d971dd1e8da22cec0e83d00640dd0cab7a911527))
* **settings:** the launch modal offered a value that kills the launch ([#60](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/60)) ([d9daade](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d9daade84f704e2a49b49ba2f75e3da9a0687c9c))
* six audit findings, including two regressions from tonight ([#104](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/104)) ([7e3386f](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/7e3386fa62de3c866292fa87dbebc132a68ca135))

## [0.2.0](https://github.com/Avarok-Cybersecurity/atlas-recipes/compare/v0.1.7...v0.2.0) (2026-08-29)


### Features

* add the local agent, telemetry, and the PyPI channel ([#26](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/26)) ([7b86068](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/7b86068b23c5813e29dfc8e0a37b2bf691989974))
* replace sparkrun with the pure-Rust atlasctl launcher ([#25](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/25)) ([9eb83b9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/9eb83b9050f985a153bc51c7050e45269551ece8))
* **run:** answer an unknown `-o` key instead of only refusing it ([#103](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/103)) ([0012bb1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/0012bb1df224bbbd665978e65982440aed5a01eb))
* **run:** say the weights are missing before pulling an image for nothing ([#100](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/100)) ([491e6ed](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/491e6ed03c29d6bc106dcc7692c4c0f09ceed280))
* **run:** warn before a GPU utilisation that has frozen a machine ([#72](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/72)) ([68493e1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/68493e1cfb1014b34975df4ac9c730271c49280b))
* **telemetry:** measure ISL and OSL, so the control page stops promising them ([#65](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/65)) ([2a2c7f7](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2a2c7f7d666d01533588afba16adb5ef8ddb9549))
* two-phase cluster launch across paired machines ([#42](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/42)) ([a94088a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/a94088a435a337e782b03f51f1e6e9c056964084))
* **windows:** native Windows support — binary, installer, and supervised agent ([#114](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/114)) ([e8b304a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e8b304aa07c190f017278657c1719bc533b2311a))


### Bug Fixes

* a destructive git escape the path guard could not see, and three more ([#117](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/117)) ([f2fcabe](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/f2fcabe83534d1cf9aa673a52db55196e8caf431))
* **agent:** a control-only node must not tell peers it can launch ([#51](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/51)) ([71ae1e9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/71ae1e99954da3330c44e558e647d19ce50cd095))
* **agent:** four ways the installed agent misbehaved on a user's machine ([#119](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/119)) ([e5fc285](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e5fc285d4a08056e4a8f6df2a243d97bb0c60e9d))
* **cluster:** choose, verify and pin the link the collective runs on ([#45](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/45)) ([99d78bb](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/99d78bb63a0e078e660da6ca818fec0bafbed6de))
* **docker:** mark what the renderer wrote, do not guess it from the text ([#90](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/90)) ([76e6043](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/76e60430821755be5b37a7212a5867eda39a5d42))
* **flags:** claim the nine recipe settings that never reached the engine ([#61](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/61)) ([d7f9ea2](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d7f9ea26692d999d499bacf9835b181f9ac33fa4))
* give every crate a literal version so release-please can read them ([#28](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/28)) ([249f5a2](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/249f5a287f3f2f424c3e3071c2a3c072e2d6af29))
* **recipe:** a recipe may not read the agent's secrets or its egress policy ([#91](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/91)) ([9fe4b3a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/9fe4b3a63097163f683a8cb18211c2f8a483fe77))
* **recipe:** refuse a field that would be read as an option ([#93](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/93)) ([6487c66](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/6487c669568d279c0e086ee78bdddd57effec4f1))
* **recipe:** suggest the recipe you meant, not the three that sort first ([#96](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/96)) ([e88874d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e88874d3b2ffca70537ace311624b8f2f9d54508))
* **registry:** a scope should not cost the operator the suggestion ([#111](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/111)) ([5b6eb5e](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/5b6eb5e7d670d19ad1e145212f3092ea9c4c8644))
* **registry:** a scoped ref cannot walk out of the recipe cache ([#92](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/92)) ([46eccef](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/46eccef8c07965e03544b29c9da057efbc2f6d30))
* **run:** guard the endpoint port against the engine's own default ([#68](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/68)) ([9189d8c](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/9189d8c28eb9712f1aa91ae3db717764891c4f3a))
* **run:** the CLI never checked a `-o` value against its declared bound ([#102](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/102)) ([c3ae320](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/c3ae320362b3833ea1c6ae433c6825450d34b443))
* **secretfile:** canonicalising every write raced Windows file replacement ([#132](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/132)) ([efe1325](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/efe132585c2756a28d04f3e1a3126b1db60ece31))
* **security:** parse the rendezvous address before it reaches argv ([#44](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/44)) ([3d48fe6](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/3d48fe60bb1e6b8e845f8db20946d8189b0841c4))
* **settings:** `port` is the TCP domain, not the IANA registered range ([#105](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/105)) ([d971dd1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d971dd1e8da22cec0e83d00640dd0cab7a911527))
* **settings:** the launch modal offered a value that kills the launch ([#60](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/60)) ([d9daade](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d9daade84f704e2a49b49ba2f75e3da9a0687c9c))
* six audit findings, including two regressions from tonight ([#104](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/104)) ([7e3386f](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/7e3386fa62de3c866292fa87dbebc132a68ca135))
