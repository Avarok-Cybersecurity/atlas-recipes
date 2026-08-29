# Changelog

## [0.2.0](https://github.com/Avarok-Cybersecurity/atlas-recipes/compare/v0.1.7...v0.2.0) (2026-08-29)


### Features

* add the local agent, telemetry, and the PyPI channel ([#26](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/26)) ([7b86068](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/7b86068b23c5813e29dfc8e0a37b2bf691989974))
* **agent:** make a worker node work — onboarding, pairing, and a legible failure ([#56](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/56)) ([e787a66](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e787a6694ff8c4e55d8095ff856bdb488f0b40d9))
* **agent:** report what accelerator a machine actually has ([#59](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/59)) ([88f00c5](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/88f00c56ced72d3d141cf1dc63c015aad32a7c21))
* **doctor:** report a disk with no room for an image and a model ([#110](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/110)) ([5222585](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/5222585c3fc3f71a46b9f5048b7f04780a4d4cac))
* fleet control plane — discovery, pairing, and multi-node agents ([#40](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/40)) ([d181b56](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d181b56e53859285820cb0ed73680334f8ab77df))
* **pairing:** trust is written after the words are compared, not before ([#63](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/63)) ([42ec9c9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/42ec9c958b4d1d960ba8cc1804b731688c1603ac))
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
* **agent:** make the installer an upgrader, and a starter ([#112](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/112)) ([6f0d8f9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/6f0d8f9f88d8fb12d6cd841cd3c1a295a7123315))
* **agent:** stop reporting "could not look" as "nothing there" ([#75](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/75)) ([c832f0a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/c832f0a6400e61cb57f1ae03388d64599a48c225))
* **cluster:** a cluster that died told nobody ([#130](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/130)) ([2e1c498](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2e1c4981eb82a4188b5335e12b1b159454a34216))
* **cluster:** a reservation whose head vanished bricked the machine forever ([#124](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/124)) ([3536607](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/35366072fc48afdf2ced7a7e6b164aa6bb19b53f))
* **cluster:** a stop that reached nobody reported every rank stopped ([#126](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/126)) ([0e0b77d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/0e0b77da7920f9ebe551f9a9c29317503b0af103))
* **cluster:** choose, verify and pin the link the collective runs on ([#45](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/45)) ([99d78bb](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/99d78bb63a0e078e660da6ca818fec0bafbed6de))
* **doctor:** "agent: not running" did not say which port it asked ([#140](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/140)) ([cd2cff6](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/cd2cff615afc33eaf31ebf182bbeb73659946a51))
* **doctor:** an unreadable interface list is not a machine with no addresses ([#69](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/69)) ([5881746](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/58817462a7e5fde606073efe0269fe4676dff259))
* **doctor:** exit non-zero when problems were found ([#97](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/97)) ([36cf651](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/36cf651dfd1997c6186ba9c2b199c3e543da90c1))
* **doctor:** the disk check measured the cwd, not where models land ([#141](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/141)) ([2faddd3](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2faddd319d3e18b265f8a25905400d7b49e79646))
* four defects an audit found in tonight's own fixes ([#145](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/145)) ([1554ae6](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/1554ae63576f2bc1ceebf5dafa1de09efeb1bb64))
* four more defects, this time in the fixes-of-the-fixes ([#146](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/146)) ([279cbdf](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/279cbdfecf43ca46aef156815e52221d1cc4676e))
* give every crate a literal version so release-please can read them ([#28](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/28)) ([249f5a2](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/249f5a287f3f2f424c3e3071c2a3c072e2d6af29))
* **join:** stop telling the operator a code has the length it needs ([#83](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/83)) ([13e17f7](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/13e17f7588ff32a66cfb9afc0ee11355b3b44e3e))
* **join:** try every address the inviter offered, without spending its attempts ([#76](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/76)) ([b6e2961](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/b6e2961cdb4864e410e8ad65acd35d7dc93b8038))
* **launch:** refuse remotely what the CLI only cautions about ([#87](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/87)) ([5e84474](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/5e844740d99dfb5fc66a511f508c6daba5474199))
* **lifecycle:** say "not running" instead of quoting the docker daemon ([#98](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/98)) ([a023034](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/a023034978139bd4c5d82e03a7925308588da337))
* **logs:** find the container `run` actually started ([#142](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/142)) ([9ded867](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/9ded867515fdf9fbf1c6a642de68db4dd60128b1))
* **messages:** strip source indentation baked into two operator-facing strings ([#67](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/67)) ([9eb8fe3](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/9eb8fe3ba065017413898429159109f62b1b4d4d))
* **pair:** a headless agent was reported as no agent at all ([#143](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/143)) ([53d9629](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/53d9629a203d315306a32e2b9574b6c67e78cea3))
* **pair:** print a command the other machine can actually run ([#77](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/77)) ([74c7657](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/74c765705a012299e6829f94124b285de93fb859))
* **pair:** sanitise the peer's name before it reaches a terminal ([#86](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/86)) ([e63441d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e63441d7005d2f9bba2220115aefdb837dbc0ae8))
* **pair:** say that the other machine has to accept too ([#79](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/79)) ([da0decb](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/da0decbe679db88f795d2be65935bfc53be15650))
* **peer:** check the pairing code before opening a socket ([#101](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/101)) ([770837b](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/770837b81e8e11ea5cfb1e483e262270cdc5b910))
* **rank:** a rank may only stop a container this fleet launched ([#94](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/94)) ([105ce8e](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/105ce8ead67c13dd666c8f6b69303284b03e5840))
* **rankservice:** the reservation TTL panicked on Windows, breaking main ([#128](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/128)) ([537b01e](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/537b01e1708613a8f848e8519a8ee28b0a6d4a21))
* **registry add:** re-adding a registry you removed no longer fails on git ([#70](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/70)) ([af9f9dd](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/af9f9dd3dc1794f9fe9c5970a0a580c453b3e54c))
* **release:** stale manifest split the versions; bump never reached the requirements ([#47](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/47)) ([7c116f2](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/7c116f21b57912271b398a5267b41115a9c9b59a))
* **run:** guard the endpoint port against the engine's own default ([#68](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/68)) ([9189d8c](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/9189d8c28eb9712f1aa91ae3db717764891c4f3a))
* **run:** the CLI never checked a `-o` value against its declared bound ([#102](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/102)) ([c3ae320](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/c3ae320362b3833ea1c6ae433c6825450d34b443))
* **run:** the weights pre-flight must not block a launch that brings its own ([#106](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/106)) ([6e046df](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/6e046df1e2d0446f1de042053ec884fd10f6f085))
* **run:** warn about a full disk before the pull, not after ([#116](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/116)) ([df676f6](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/df676f682da9068948722bd7c322e45f1388913c))
* **scrape:** a chunk length that lands mid-character must not panic ([#95](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/95)) ([405d6e0](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/405d6e032bf39c2e1981833825f556803e607dab))
* six audit findings, including two regressions from tonight ([#104](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/104)) ([7e3386f](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/7e3386fa62de3c866292fa87dbebc132a68ca135))
* **stop:** a stop that did not stop anything is not a success ([#66](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/66)) ([c9bfe89](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/c9bfe89e657993f5dd726bd042a1088e538c3a51))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * atlasctl-protocol bumped from 0.4.1 to 0.4.2
