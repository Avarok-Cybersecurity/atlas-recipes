# Changelog

## [0.4.1](https://github.com/Avarok-Cybersecurity/atlas-recipes/compare/v0.4.0...v0.4.1) (2026-08-28)


### Bug Fixes

* **agent:** name what is holding the peer port ([#80](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/80)) ([b7ca139](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/b7ca13942929b08f007fdb2b35008e028c48206f))
* **fleet:** a beacon must not rewrite a trusted peer's address ([#82](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/82)) ([32b9669](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/32b96695600735e01545fc8df8b81c5c4418c942))
* **join:** stop telling the operator a code has the length it needs ([#83](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/83)) ([13e17f7](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/13e17f7588ff32a66cfb9afc0ee11355b3b44e3e))
* **pair:** say that the other machine has to accept too ([#79](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/79)) ([da0decb](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/da0decbe679db88f795d2be65935bfc53be15650))
* pin mtp_gate=force on the two gate-backing recipes ([#16](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/16)) ([ce399ba](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/ce399ba7d1d335e6b2add767e625236ebe0b0a3b))

## [0.4.0](https://github.com/Avarok-Cybersecurity/atlas-recipes/compare/v0.3.0...v0.4.0) (2026-08-28)


### Features

* **pairing:** trust is written after the words are compared, not before ([#63](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/63)) ([42ec9c9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/42ec9c958b4d1d960ba8cc1804b731688c1603ac))
* **run:** warn before a GPU utilisation that has frozen a machine ([#72](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/72)) ([68493e1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/68493e1cfb1014b34975df4ac9c730271c49280b))
* **telemetry:** measure ISL and OSL, so the control page stops promising them ([#65](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/65)) ([2a2c7f7](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2a2c7f7d666d01533588afba16adb5ef8ddb9549))


### Bug Fixes

* **agent:** stop reporting "could not look" as "nothing there" ([#75](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/75)) ([c832f0a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/c832f0a6400e61cb57f1ae03388d64599a48c225))
* **doctor:** an unreadable interface list is not a machine with no addresses ([#69](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/69)) ([5881746](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/58817462a7e5fde606073efe0269fe4676dff259))
* **errors:** a failure reply carries the cause, not only the attempt ([#71](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/71)) ([1335721](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/1335721c8b7ff88c096fb389e924002cb93c0407))
* **fleet:** bound the sightings table against a beacon flood ([#73](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/73)) ([44057b9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/44057b98feeda4ab2e631f1da91463b27fe52731))
* **join:** try every address the inviter offered, without spending its attempts ([#76](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/76)) ([b6e2961](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/b6e2961cdb4864e410e8ad65acd35d7dc93b8038))
* **messages:** strip source indentation baked into two operator-facing strings ([#67](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/67)) ([9eb8fe3](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/9eb8fe3ba065017413898429159109f62b1b4d4d))
* **pair:** print a command the other machine can actually run ([#77](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/77)) ([74c7657](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/74c765705a012299e6829f94124b285de93fb859))
* **protocol:** name the relay in its own refusal ([#74](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/74)) ([289fe8f](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/289fe8f8747916d7348ae3fec3a3133ae58d9213))
* **registry add:** re-adding a registry you removed no longer fails on git ([#70](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/70)) ([af9f9dd](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/af9f9dd3dc1794f9fe9c5970a0a580c453b3e54c))
* **resolve:** an IPv6 literal is not a host with a port ([#78](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/78)) ([53010c6](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/53010c646bb171179d6808ab7ac69e9c301f7307))
* **run:** guard the endpoint port against the engine's own default ([#68](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/68)) ([9189d8c](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/9189d8c28eb9712f1aa91ae3db717764891c4f3a))
* **stop:** a stop that did not stop anything is not a success ([#66](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/66)) ([c9bfe89](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/c9bfe89e657993f5dd726bd042a1088e538c3a51))

## [0.3.0](https://github.com/Avarok-Cybersecurity/atlas-recipes/compare/v0.2.0...v0.3.0) (2026-08-26)


### Features

* **agent:** make a worker node work — onboarding, pairing, and a legible failure ([#56](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/56)) ([e787a66](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e787a6694ff8c4e55d8095ff856bdb488f0b40d9))
* **agent:** report what accelerator a machine actually has ([#59](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/59)) ([88f00c5](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/88f00c56ced72d3d141cf1dc63c015aad32a7c21))


### Bug Fixes

* **cli:** say why a join was refused, not which TLS alert arrived ([#58](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/58)) ([2d35e1f](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2d35e1f37f368d35723d2c49cbaae2362e9a7f26))
* **flags:** claim the nine recipe settings that never reached the engine ([#61](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/61)) ([d7f9ea2](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d7f9ea26692d999d499bacf9835b181f9ac33fa4))
* **release:** build the musl wheel where a musl compiler exists ([#54](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/54)) ([1f4224b](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/1f4224be485cdf05db076a0408c9baec491e3555))
* **release:** the already-published guard never fired ([#57](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/57)) ([e206079](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e206079ee1c683a731f730e470724dc4ba39e146))
* **settings:** the launch modal offered a value that kills the launch ([#60](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/60)) ([d9daade](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d9daade84f704e2a49b49ba2f75e3da9a0687c9c))

## [0.2.0](https://github.com/Avarok-Cybersecurity/atlas-recipes/compare/v0.1.3...v0.2.0) (2026-08-26)


### Features

* fleet control plane — discovery, pairing, and multi-node agents ([#40](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/40)) ([d181b56](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d181b56e53859285820cb0ed73680334f8ab77df))
* two-phase cluster launch across paired machines ([#42](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/42)) ([a94088a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/a94088a435a337e782b03f51f1e6e9c056964084))


### Bug Fixes

* **agent:** a control-only node must not tell peers it can launch ([#51](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/51)) ([71ae1e9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/71ae1e99954da3330c44e558e647d19ce50cd095))
* **cluster:** choose, verify and pin the link the collective runs on ([#45](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/45)) ([99d78bb](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/99d78bb63a0e078e660da6ca818fec0bafbed6de))
* **release:** linked-versions was grouping nothing ([#48](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/48)) ([774f8f7](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/774f8f72b612f2e75a07838dce1726e24cd151e6))
* **release:** my own check blocked the repair it was meant to protect ([#50](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/50)) ([816419e](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/816419e7a83bcf8610a7b900f15125a6aa99e41a))
* **release:** repair a manifest entry release-please left behind ([#52](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/52)) ([e0172ba](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e0172ba02df90c3104b386e8dedb763fa0d88023))
* **release:** repair the requirements where the lock is already repaired ([#49](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/49)) ([7387328](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/738732890a3da6599882cf7b90d1296edac5d05f))
* **release:** stale manifest split the versions; bump never reached the requirements ([#47](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/47)) ([7c116f2](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/7c116f21b57912271b398a5267b41115a9c9b59a))
* **release:** the repair step threw away the manifest fix it had just made ([#53](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/53)) ([d690a6b](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d690a6b7c424a4dd1571747af6fe47c9bfe1e111))
* **release:** workspace dep version drift blocks every release ([#46](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/46)) ([2472171](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2472171c1b6f29b9eb49f1ed4eeadba48d3d63c6))
* **security:** parse the rendezvous address before it reaches argv ([#44](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/44)) ([3d48fe6](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/3d48fe60bb1e6b8e845f8db20946d8189b0841c4))

## [0.1.3](https://github.com/Avarok-Cybersecurity/atlas-recipes/compare/v0.1.2...v0.1.3) (2026-08-26)


### Bug Fixes

* cross builds and a publish job that could never have worked ([#37](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/37)) ([c4f03da](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/c4f03dac4c72e60305a996e8b507a23cf7a15bfb))
* let cargo order the workspace publish, and stop calling a script from the tag ([#39](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/39)) ([5f1dc6f](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/5f1dc6f33c181e6c5b483681a815480b834cfdff))

## [0.1.2](https://github.com/Avarok-Cybersecurity/atlas-recipes/compare/v0.1.1...v0.1.2) (2026-08-25)


### Bug Fixes

* a skipped job skips its whole needs chain, not one hop ([#34](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/34)) ([acb281e](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/acb281e519a965f84e721241d2f6e948ad387427))
* enforce the release PR's lock on the artifact, not on release-please ([#36](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/36)) ([0432d85](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/0432d85b4dc7857c0398728d66e2be940a36ed75))
* keep Cargo.lock in step with the versions release-please writes ([#31](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/31)) ([7d4af1b](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/7d4af1b6c1c1503057e79a99f8334d58e6dc7e2a))
* make the manual release path actually run ([#33](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/33)) ([95e6922](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/95e6922d0ef95957eb8d22c2d8ecd2a18d04185d))
* the release PR gets no CI, so it kept cutting tags that cannot build ([#35](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/35)) ([e452a5e](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e452a5eeb07a43f4fd863e5855bf15d8a35a0429))

## [0.1.1](https://github.com/Avarok-Cybersecurity/atlas-recipes/compare/v0.1.0...v0.1.1) (2026-08-25)


### Bug Fixes

* give every crate a literal version so release-please can read them ([#28](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/28)) ([249f5a2](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/249f5a287f3f2f424c3e3071c2a3c072e2d6af29))
* release the workspace as one version, and give it a baseline tag ([#30](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/30)) ([2199ef2](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2199ef27eef5beb502057547e1ba79379a98ae8b))
