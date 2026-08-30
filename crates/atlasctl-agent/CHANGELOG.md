# Changelog

## [0.5.0](https://github.com/Avarok-Cybersecurity/atlas-recipes/compare/v0.4.0...v0.5.0) (2026-08-30)


### Features

* **windows:** native Windows support — binary, installer, and supervised agent ([#114](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/114)) ([e8b304a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e8b304aa07c190f017278657c1719bc533b2311a))


### Bug Fixes

* a destructive git escape the path guard could not see, and three more ([#117](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/117)) ([f2fcabe](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/f2fcabe83534d1cf9aa673a52db55196e8caf431))
* **agent:** four ways the installed agent misbehaved on a user's machine ([#119](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/119)) ([e5fc285](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e5fc285d4a08056e4a8f6df2a243d97bb0c60e9d))
* **agent:** launches use the blocking pool, not a runtime worker ([#149](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/149)) ([1eaf484](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/1eaf48418e7332f742151acc3c2ae35ee320c452))
* **agent:** make the installer an upgrader, and a starter ([#112](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/112)) ([6f0d8f9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/6f0d8f9f88d8fb12d6cd841cd3c1a295a7123315))
* **agent:** name what is holding the peer port ([#80](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/80)) ([b7ca139](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/b7ca13942929b08f007fdb2b35008e028c48206f))
* **cache:** a directory is not a downloaded model ([#150](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/150)) ([b83b327](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/b83b32744c7eb59ddc09f9e32f4f272c615103db))
* **cache:** a nested weight file is still a weight file ([#154](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/154)) ([1ff0854](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/1ff085413a8002ad7925859e1427b0546781233f))
* **cluster:** a cluster that died told nobody ([#130](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/130)) ([2e1c498](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2e1c4981eb82a4188b5335e12b1b159454a34216))
* **cluster:** a reservation whose head vanished bricked the machine forever ([#124](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/124)) ([3536607](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/35366072fc48afdf2ced7a7e6b164aa6bb19b53f))
* **cluster:** a stop that reached nobody reported every rank stopped ([#126](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/126)) ([0e0b77d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/0e0b77da7920f9ebe551f9a9c29317503b0af103))
* **cluster:** commit gets its own answer budget, not the 5s dial bound ([#129](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/129)) ([3002c62](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/3002c62fa7fbd313d8e7a929a58739218e992b3b))
* **cluster:** refuse a second cluster while one is running ([#131](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/131)) ([986219b](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/986219bdfcd77a283f58bf159a7ff8779f7c8756))
* **cluster:** the endpoint fallback named a port the engine never uses ([#139](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/139)) ([2f652a7](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2f652a7af87bc3c7ea687d761b85f483157fec88))
* **fleet:** a beacon must not rewrite a trusted peer's address ([#82](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/82)) ([32b9669](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/32b96695600735e01545fc8df8b81c5c4418c942))
* **fleet:** dial the port the peer advertised, not our own ([#85](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/85)) ([edd3c84](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/edd3c8497e60df66815a80cfb9bffd45519a7acc))
* **join:** say so when minting a code nothing can accept ([#157](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/157)) ([da3fba1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/da3fba18061bc874357adb84031a7ee067c8779e))
* **join:** stop blaming the code when nothing answered ([#156](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/156)) ([eb54864](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/eb5486468e4de7d0ba0b848a3c26bec4018e088b))
* **launcher:** stop could not reach a cluster rank's container ([#133](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/133)) ([e8897a1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e8897a1338bffb7f6fbf226551a8edf434ef13bc))
* **launch:** refuse remotely what the CLI only cautions about ([#87](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/87)) ([5e84474](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/5e844740d99dfb5fc66a511f508c6daba5474199))
* **onboarding:** stop contradicting the operator ([#151](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/151)) ([fad9e17](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/fad9e1755ba5b9a6f8dfd86fbf85c8e65288178b))
* **pair:** sanitise the peer's name before it reaches a terminal ([#86](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/86)) ([e63441d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e63441d7005d2f9bba2220115aefdb837dbc0ae8))
* **peer:** sanitise the refusal, and check the version both ways ([#89](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/89)) ([54da03d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/54da03d8e346bacb1fcd32c915cb6a544da86ec4))
* **reach:** an unasked question is not an unanswered one ([#162](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/162)) ([242e8ae](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/242e8aefb0fbbe4ca934b9c563b716b09aa84686))
* **settings:** `port` is the TCP domain, not the IANA registered range ([#105](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/105)) ([d971dd1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d971dd1e8da22cec0e83d00640dd0cab7a911527))

## [0.4.0](https://github.com/Avarok-Cybersecurity/atlas-recipes/compare/v0.3.0...v0.4.0) (2026-08-29)


### Features

* **pairing:** trust is written after the words are compared, not before ([#63](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/63)) ([42ec9c9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/42ec9c958b4d1d960ba8cc1804b731688c1603ac))
* **telemetry:** measure ISL and OSL, so the control page stops promising them ([#65](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/65)) ([2a2c7f7](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2a2c7f7d666d01533588afba16adb5ef8ddb9549))
* **windows:** native Windows support — binary, installer, and supervised agent ([#114](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/114)) ([e8b304a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e8b304aa07c190f017278657c1719bc533b2311a))


### Bug Fixes

* a destructive git escape the path guard could not see, and three more ([#117](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/117)) ([f2fcabe](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/f2fcabe83534d1cf9aa673a52db55196e8caf431))
* **agent:** four ways the installed agent misbehaved on a user's machine ([#119](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/119)) ([e5fc285](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e5fc285d4a08056e4a8f6df2a243d97bb0c60e9d))
* **agent:** launches use the blocking pool, not a runtime worker ([#149](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/149)) ([1eaf484](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/1eaf48418e7332f742151acc3c2ae35ee320c452))
* **agent:** make the installer an upgrader, and a starter ([#112](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/112)) ([6f0d8f9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/6f0d8f9f88d8fb12d6cd841cd3c1a295a7123315))
* **agent:** name what is holding the peer port ([#80](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/80)) ([b7ca139](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/b7ca13942929b08f007fdb2b35008e028c48206f))
* **agent:** stop reporting "could not look" as "nothing there" ([#75](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/75)) ([c832f0a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/c832f0a6400e61cb57f1ae03388d64599a48c225))
* **cache:** a directory is not a downloaded model ([#150](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/150)) ([b83b327](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/b83b32744c7eb59ddc09f9e32f4f272c615103db))
* **cache:** a nested weight file is still a weight file ([#154](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/154)) ([1ff0854](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/1ff085413a8002ad7925859e1427b0546781233f))
* **cluster:** a cluster that died told nobody ([#130](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/130)) ([2e1c498](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2e1c4981eb82a4188b5335e12b1b159454a34216))
* **cluster:** a reservation whose head vanished bricked the machine forever ([#124](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/124)) ([3536607](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/35366072fc48afdf2ced7a7e6b164aa6bb19b53f))
* **cluster:** a stop that reached nobody reported every rank stopped ([#126](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/126)) ([0e0b77d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/0e0b77da7920f9ebe551f9a9c29317503b0af103))
* **cluster:** commit gets its own answer budget, not the 5s dial bound ([#129](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/129)) ([3002c62](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/3002c62fa7fbd313d8e7a929a58739218e992b3b))
* **cluster:** refuse a second cluster while one is running ([#131](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/131)) ([986219b](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/986219bdfcd77a283f58bf159a7ff8779f7c8756))
* **cluster:** the endpoint fallback named a port the engine never uses ([#139](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/139)) ([2f652a7](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2f652a7af87bc3c7ea687d761b85f483157fec88))
* **errors:** a failure reply carries the cause, not only the attempt ([#71](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/71)) ([1335721](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/1335721c8b7ff88c096fb389e924002cb93c0407))
* **fleet:** a beacon must not rewrite a trusted peer's address ([#82](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/82)) ([32b9669](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/32b96695600735e01545fc8df8b81c5c4418c942))
* **fleet:** bound the sightings table against a beacon flood ([#73](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/73)) ([44057b9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/44057b98feeda4ab2e631f1da91463b27fe52731))
* **fleet:** dial the port the peer advertised, not our own ([#85](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/85)) ([edd3c84](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/edd3c8497e60df66815a80cfb9bffd45519a7acc))
* **join:** say so when minting a code nothing can accept ([#157](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/157)) ([da3fba1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/da3fba18061bc874357adb84031a7ee067c8779e))
* **join:** stop blaming the code when nothing answered ([#156](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/156)) ([eb54864](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/eb5486468e4de7d0ba0b848a3c26bec4018e088b))
* **join:** try every address the inviter offered, without spending its attempts ([#76](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/76)) ([b6e2961](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/b6e2961cdb4864e410e8ad65acd35d7dc93b8038))
* **launcher:** stop could not reach a cluster rank's container ([#133](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/133)) ([e8897a1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e8897a1338bffb7f6fbf226551a8edf434ef13bc))
* **launch:** refuse remotely what the CLI only cautions about ([#87](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/87)) ([5e84474](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/5e844740d99dfb5fc66a511f508c6daba5474199))
* **messages:** strip source indentation baked into two operator-facing strings ([#67](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/67)) ([9eb8fe3](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/9eb8fe3ba065017413898429159109f62b1b4d4d))
* **onboarding:** stop contradicting the operator ([#151](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/151)) ([fad9e17](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/fad9e1755ba5b9a6f8dfd86fbf85c8e65288178b))
* **pair:** sanitise the peer's name before it reaches a terminal ([#86](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/86)) ([e63441d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e63441d7005d2f9bba2220115aefdb837dbc0ae8))
* **peer:** sanitise the refusal, and check the version both ways ([#89](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/89)) ([54da03d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/54da03d8e346bacb1fcd32c915cb6a544da86ec4))
* **protocol:** name the relay in its own refusal ([#74](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/74)) ([289fe8f](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/289fe8f8747916d7348ae3fec3a3133ae58d9213))
* **reach:** an unasked question is not an unanswered one ([#162](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/162)) ([242e8ae](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/242e8aefb0fbbe4ca934b9c563b716b09aa84686))
* **resolve:** an IPv6 literal is not a host with a port ([#78](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/78)) ([53010c6](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/53010c646bb171179d6808ab7ac69e9c301f7307))
* **settings:** `port` is the TCP domain, not the IANA registered range ([#105](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/105)) ([d971dd1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d971dd1e8da22cec0e83d00640dd0cab7a911527))

## [0.3.0](https://github.com/Avarok-Cybersecurity/atlas-recipes/compare/v0.2.0...v0.3.0) (2026-08-29)


### Features

* **agent:** make a worker node work — onboarding, pairing, and a legible failure ([#56](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/56)) ([e787a66](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e787a6694ff8c4e55d8095ff856bdb488f0b40d9))
* **agent:** report what accelerator a machine actually has ([#59](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/59)) ([88f00c5](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/88f00c56ced72d3d141cf1dc63c015aad32a7c21))
* **pairing:** trust is written after the words are compared, not before ([#63](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/63)) ([42ec9c9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/42ec9c958b4d1d960ba8cc1804b731688c1603ac))
* **telemetry:** measure ISL and OSL, so the control page stops promising them ([#65](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/65)) ([2a2c7f7](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2a2c7f7d666d01533588afba16adb5ef8ddb9549))
* **windows:** native Windows support — binary, installer, and supervised agent ([#114](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/114)) ([e8b304a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e8b304aa07c190f017278657c1719bc533b2311a))


### Bug Fixes

* a destructive git escape the path guard could not see, and three more ([#117](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/117)) ([f2fcabe](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/f2fcabe83534d1cf9aa673a52db55196e8caf431))
* **agent:** four ways the installed agent misbehaved on a user's machine ([#119](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/119)) ([e5fc285](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e5fc285d4a08056e4a8f6df2a243d97bb0c60e9d))
* **agent:** launches use the blocking pool, not a runtime worker ([#149](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/149)) ([1eaf484](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/1eaf48418e7332f742151acc3c2ae35ee320c452))
* **agent:** make the installer an upgrader, and a starter ([#112](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/112)) ([6f0d8f9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/6f0d8f9f88d8fb12d6cd841cd3c1a295a7123315))
* **agent:** name what is holding the peer port ([#80](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/80)) ([b7ca139](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/b7ca13942929b08f007fdb2b35008e028c48206f))
* **agent:** stop reporting "could not look" as "nothing there" ([#75](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/75)) ([c832f0a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/c832f0a6400e61cb57f1ae03388d64599a48c225))
* **cache:** a directory is not a downloaded model ([#150](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/150)) ([b83b327](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/b83b32744c7eb59ddc09f9e32f4f272c615103db))
* **cache:** a nested weight file is still a weight file ([#154](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/154)) ([1ff0854](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/1ff085413a8002ad7925859e1427b0546781233f))
* **cli:** say why a join was refused, not which TLS alert arrived ([#58](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/58)) ([2d35e1f](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2d35e1f37f368d35723d2c49cbaae2362e9a7f26))
* **cluster:** a cluster that died told nobody ([#130](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/130)) ([2e1c498](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2e1c4981eb82a4188b5335e12b1b159454a34216))
* **cluster:** a reservation whose head vanished bricked the machine forever ([#124](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/124)) ([3536607](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/35366072fc48afdf2ced7a7e6b164aa6bb19b53f))
* **cluster:** a stop that reached nobody reported every rank stopped ([#126](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/126)) ([0e0b77d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/0e0b77da7920f9ebe551f9a9c29317503b0af103))
* **cluster:** commit gets its own answer budget, not the 5s dial bound ([#129](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/129)) ([3002c62](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/3002c62fa7fbd313d8e7a929a58739218e992b3b))
* **cluster:** refuse a second cluster while one is running ([#131](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/131)) ([986219b](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/986219bdfcd77a283f58bf159a7ff8779f7c8756))
* **cluster:** the endpoint fallback named a port the engine never uses ([#139](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/139)) ([2f652a7](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2f652a7af87bc3c7ea687d761b85f483157fec88))
* **errors:** a failure reply carries the cause, not only the attempt ([#71](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/71)) ([1335721](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/1335721c8b7ff88c096fb389e924002cb93c0407))
* **fleet:** a beacon must not rewrite a trusted peer's address ([#82](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/82)) ([32b9669](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/32b96695600735e01545fc8df8b81c5c4418c942))
* **fleet:** bound the sightings table against a beacon flood ([#73](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/73)) ([44057b9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/44057b98feeda4ab2e631f1da91463b27fe52731))
* **fleet:** dial the port the peer advertised, not our own ([#85](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/85)) ([edd3c84](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/edd3c8497e60df66815a80cfb9bffd45519a7acc))
* **join:** say so when minting a code nothing can accept ([#157](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/157)) ([da3fba1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/da3fba18061bc874357adb84031a7ee067c8779e))
* **join:** stop blaming the code when nothing answered ([#156](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/156)) ([eb54864](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/eb5486468e4de7d0ba0b848a3c26bec4018e088b))
* **join:** try every address the inviter offered, without spending its attempts ([#76](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/76)) ([b6e2961](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/b6e2961cdb4864e410e8ad65acd35d7dc93b8038))
* **launcher:** stop could not reach a cluster rank's container ([#133](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/133)) ([e8897a1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e8897a1338bffb7f6fbf226551a8edf434ef13bc))
* **launch:** refuse remotely what the CLI only cautions about ([#87](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/87)) ([5e84474](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/5e844740d99dfb5fc66a511f508c6daba5474199))
* **messages:** strip source indentation baked into two operator-facing strings ([#67](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/67)) ([9eb8fe3](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/9eb8fe3ba065017413898429159109f62b1b4d4d))
* **onboarding:** stop contradicting the operator ([#151](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/151)) ([fad9e17](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/fad9e1755ba5b9a6f8dfd86fbf85c8e65288178b))
* **pair:** sanitise the peer's name before it reaches a terminal ([#86](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/86)) ([e63441d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e63441d7005d2f9bba2220115aefdb837dbc0ae8))
* **peer:** sanitise the refusal, and check the version both ways ([#89](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/89)) ([54da03d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/54da03d8e346bacb1fcd32c915cb6a544da86ec4))
* **protocol:** name the relay in its own refusal ([#74](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/74)) ([289fe8f](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/289fe8f8747916d7348ae3fec3a3133ae58d9213))
* **resolve:** an IPv6 literal is not a host with a port ([#78](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/78)) ([53010c6](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/53010c646bb171179d6808ab7ac69e9c301f7307))
* **settings:** `port` is the TCP domain, not the IANA registered range ([#105](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/105)) ([d971dd1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d971dd1e8da22cec0e83d00640dd0cab7a911527))

## [0.2.0](https://github.com/Avarok-Cybersecurity/atlas-recipes/compare/v0.1.7...v0.2.0) (2026-08-29)


### Features

* add the local agent, telemetry, and the PyPI channel ([#26](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/26)) ([7b86068](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/7b86068b23c5813e29dfc8e0a37b2bf691989974))
* **agent:** make a worker node work — onboarding, pairing, and a legible failure ([#56](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/56)) ([e787a66](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e787a6694ff8c4e55d8095ff856bdb488f0b40d9))
* **agent:** report what accelerator a machine actually has ([#59](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/59)) ([88f00c5](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/88f00c56ced72d3d141cf1dc63c015aad32a7c21))
* fleet control plane — discovery, pairing, and multi-node agents ([#40](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/40)) ([d181b56](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d181b56e53859285820cb0ed73680334f8ab77df))
* **pairing:** trust is written after the words are compared, not before ([#63](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/63)) ([42ec9c9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/42ec9c958b4d1d960ba8cc1804b731688c1603ac))
* **telemetry:** measure ISL and OSL, so the control page stops promising them ([#65](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/65)) ([2a2c7f7](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2a2c7f7d666d01533588afba16adb5ef8ddb9549))
* two-phase cluster launch across paired machines ([#42](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/42)) ([a94088a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/a94088a435a337e782b03f51f1e6e9c056964084))
* **windows:** native Windows support — binary, installer, and supervised agent ([#114](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/114)) ([e8b304a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e8b304aa07c190f017278657c1719bc533b2311a))


### Bug Fixes

* a destructive git escape the path guard could not see, and three more ([#117](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/117)) ([f2fcabe](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/f2fcabe83534d1cf9aa673a52db55196e8caf431))
* **agent:** a control-only node must not tell peers it can launch ([#51](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/51)) ([71ae1e9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/71ae1e99954da3330c44e558e647d19ce50cd095))
* **agent:** four ways the installed agent misbehaved on a user's machine ([#119](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/119)) ([e5fc285](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e5fc285d4a08056e4a8f6df2a243d97bb0c60e9d))
* **agent:** make the installer an upgrader, and a starter ([#112](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/112)) ([6f0d8f9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/6f0d8f9f88d8fb12d6cd841cd3c1a295a7123315))
* **agent:** name what is holding the peer port ([#80](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/80)) ([b7ca139](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/b7ca13942929b08f007fdb2b35008e028c48206f))
* **agent:** stop reporting "could not look" as "nothing there" ([#75](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/75)) ([c832f0a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/c832f0a6400e61cb57f1ae03388d64599a48c225))
* **cli:** say why a join was refused, not which TLS alert arrived ([#58](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/58)) ([2d35e1f](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2d35e1f37f368d35723d2c49cbaae2362e9a7f26))
* **cluster:** a cluster that died told nobody ([#130](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/130)) ([2e1c498](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2e1c4981eb82a4188b5335e12b1b159454a34216))
* **cluster:** a reservation whose head vanished bricked the machine forever ([#124](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/124)) ([3536607](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/35366072fc48afdf2ced7a7e6b164aa6bb19b53f))
* **cluster:** a stop that reached nobody reported every rank stopped ([#126](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/126)) ([0e0b77d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/0e0b77da7920f9ebe551f9a9c29317503b0af103))
* **cluster:** choose, verify and pin the link the collective runs on ([#45](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/45)) ([99d78bb](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/99d78bb63a0e078e660da6ca818fec0bafbed6de))
* **cluster:** commit gets its own answer budget, not the 5s dial bound ([#129](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/129)) ([3002c62](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/3002c62fa7fbd313d8e7a929a58739218e992b3b))
* **cluster:** refuse a second cluster while one is running ([#131](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/131)) ([986219b](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/986219bdfcd77a283f58bf159a7ff8779f7c8756))
* **cluster:** the endpoint fallback named a port the engine never uses ([#139](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/139)) ([2f652a7](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2f652a7af87bc3c7ea687d761b85f483157fec88))
* **errors:** a failure reply carries the cause, not only the attempt ([#71](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/71)) ([1335721](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/1335721c8b7ff88c096fb389e924002cb93c0407))
* **fleet:** a beacon must not rewrite a trusted peer's address ([#82](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/82)) ([32b9669](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/32b96695600735e01545fc8df8b81c5c4418c942))
* **fleet:** bound the sightings table against a beacon flood ([#73](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/73)) ([44057b9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/44057b98feeda4ab2e631f1da91463b27fe52731))
* **fleet:** dial the port the peer advertised, not our own ([#85](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/85)) ([edd3c84](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/edd3c8497e60df66815a80cfb9bffd45519a7acc))
* give every crate a literal version so release-please can read them ([#28](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/28)) ([249f5a2](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/249f5a287f3f2f424c3e3071c2a3c072e2d6af29))
* **join:** try every address the inviter offered, without spending its attempts ([#76](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/76)) ([b6e2961](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/b6e2961cdb4864e410e8ad65acd35d7dc93b8038))
* **launcher:** stop could not reach a cluster rank's container ([#133](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/133)) ([e8897a1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e8897a1338bffb7f6fbf226551a8edf434ef13bc))
* **launch:** refuse remotely what the CLI only cautions about ([#87](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/87)) ([5e84474](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/5e844740d99dfb5fc66a511f508c6daba5474199))
* **messages:** strip source indentation baked into two operator-facing strings ([#67](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/67)) ([9eb8fe3](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/9eb8fe3ba065017413898429159109f62b1b4d4d))
* **pair:** sanitise the peer's name before it reaches a terminal ([#86](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/86)) ([e63441d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e63441d7005d2f9bba2220115aefdb837dbc0ae8))
* **peer:** sanitise the refusal, and check the version both ways ([#89](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/89)) ([54da03d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/54da03d8e346bacb1fcd32c915cb6a544da86ec4))
* **protocol:** name the relay in its own refusal ([#74](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/74)) ([289fe8f](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/289fe8f8747916d7348ae3fec3a3133ae58d9213))
* **resolve:** an IPv6 literal is not a host with a port ([#78](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/78)) ([53010c6](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/53010c646bb171179d6808ab7ac69e9c301f7307))
* **settings:** `port` is the TCP domain, not the IANA registered range ([#105](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/105)) ([d971dd1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d971dd1e8da22cec0e83d00640dd0cab7a911527))
