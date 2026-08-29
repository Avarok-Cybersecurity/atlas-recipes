# Changelog

## [0.6.0](https://github.com/Avarok-Cybersecurity/atlas-recipes/compare/v0.5.0...v0.6.0) (2026-08-29)


### Features

* **doctor:** report the listener other machines actually dial ([#155](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/155)) ([3caab9f](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/3caab9fe3480ebd7487b0e9b5d1f714b8b5258fb))
* **recipes:** a recipe for Qwen3.8-Flash-Next ([#120](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/120)) ([3d2db11](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/3d2db1180d7d29732554fa4a48ca2a17e8a3feb7))


### Bug Fixes

* **agent:** launches use the blocking pool, not a runtime worker ([#149](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/149)) ([1eaf484](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/1eaf48418e7332f742151acc3c2ae35ee320c452))
* **cache:** a directory is not a downloaded model ([#150](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/150)) ([b83b327](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/b83b32744c7eb59ddc09f9e32f4f272c615103db))
* **cache:** a nested weight file is still a weight file ([#154](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/154)) ([1ff0854](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/1ff085413a8002ad7925859e1427b0546781233f))
* **join:** say so when minting a code nothing can accept ([#157](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/157)) ([da3fba1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/da3fba18061bc874357adb84031a7ee067c8779e))
* **join:** stop blaming the code when nothing answered ([#156](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/156)) ([eb54864](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/eb5486468e4de7d0ba0b848a3c26bec4018e088b))
* **onboarding:** stop contradicting the operator ([#151](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/151)) ([fad9e17](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/fad9e1755ba5b9a6f8dfd86fbf85c8e65288178b))
* **release:** don't publish a release before its assets exist ([#148](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/148)) ([0b7b426](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/0b7b4262bac1e0f5de256a918cfb373789e14b5c))

## [0.5.0](https://github.com/Avarok-Cybersecurity/atlas-recipes/compare/v0.4.1...v0.5.0) (2026-08-29)


### Features

* **doctor:** report a disk with no room for an image and a model ([#110](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/110)) ([5222585](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/5222585c3fc3f71a46b9f5048b7f04780a4d4cac))
* **run:** answer an unknown `-o` key instead of only refusing it ([#103](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/103)) ([0012bb1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/0012bb1df224bbbd665978e65982440aed5a01eb))
* **run:** say the weights are missing before pulling an image for nothing ([#100](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/100)) ([491e6ed](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/491e6ed03c29d6bc106dcc7692c4c0f09ceed280))
* **windows:** native Windows support — binary, installer, and supervised agent ([#114](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/114)) ([e8b304a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e8b304aa07c190f017278657c1719bc533b2311a))


### Bug Fixes

* a destructive git escape the path guard could not see, and three more ([#117](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/117)) ([f2fcabe](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/f2fcabe83534d1cf9aa673a52db55196e8caf431))
* **agent:** four ways the installed agent misbehaved on a user's machine ([#119](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/119)) ([e5fc285](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e5fc285d4a08056e4a8f6df2a243d97bb0c60e9d))
* **agent:** make the installer an upgrader, and a starter ([#112](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/112)) ([6f0d8f9](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/6f0d8f9f88d8fb12d6cd841cd3c1a295a7123315))
* **cluster:** a cluster that died told nobody ([#130](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/130)) ([2e1c498](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2e1c4981eb82a4188b5335e12b1b159454a34216))
* **cluster:** a reservation whose head vanished bricked the machine forever ([#124](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/124)) ([3536607](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/35366072fc48afdf2ced7a7e6b164aa6bb19b53f))
* **cluster:** a stop that reached nobody reported every rank stopped ([#126](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/126)) ([0e0b77d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/0e0b77da7920f9ebe551f9a9c29317503b0af103))
* **cluster:** commit gets its own answer budget, not the 5s dial bound ([#129](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/129)) ([3002c62](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/3002c62fa7fbd313d8e7a929a58739218e992b3b))
* **cluster:** refuse a second cluster while one is running ([#131](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/131)) ([986219b](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/986219bdfcd77a283f58bf159a7ff8779f7c8756))
* **cluster:** the endpoint fallback named a port the engine never uses ([#139](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/139)) ([2f652a7](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2f652a7af87bc3c7ea687d761b85f483157fec88))
* **docker:** mark what the renderer wrote, do not guess it from the text ([#90](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/90)) ([76e6043](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/76e60430821755be5b37a7212a5867eda39a5d42))
* **doctor:** "agent: not running" did not say which port it asked ([#140](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/140)) ([cd2cff6](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/cd2cff615afc33eaf31ebf182bbeb73659946a51))
* **doctor:** exit non-zero when problems were found ([#97](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/97)) ([36cf651](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/36cf651dfd1997c6186ba9c2b199c3e543da90c1))
* **doctor:** the disk check measured the cwd, not where models land ([#141](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/141)) ([2faddd3](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/2faddd319d3e18b265f8a25905400d7b49e79646))
* **fleet:** dial the port the peer advertised, not our own ([#85](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/85)) ([edd3c84](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/edd3c8497e60df66815a80cfb9bffd45519a7acc))
* four defects an audit found in tonight's own fixes ([#145](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/145)) ([1554ae6](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/1554ae63576f2bc1ceebf5dafa1de09efeb1bb64))
* four more defects, this time in the fixes-of-the-fixes ([#146](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/146)) ([279cbdf](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/279cbdfecf43ca46aef156815e52221d1cc4676e))
* **install:** --grant-control was announced and then dropped ([#137](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/137)) ([d4d324b](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d4d324bc9549532b1f0d0d8b3268170ad305e185))
* **install.ps1:** stop leaving the operator's PowerShell session altered ([#121](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/121)) ([3f2bbb1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/3f2bbb1370a6d810f245f84b18af8f83e5cf2f1e))
* **install.sh:** stop crying wolf about provenance, and tell Windows readers the README exists for them too ([#115](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/115)) ([c9965fa](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/c9965fad312c0ee45d423a1b4b0e97ee459297bd))
* **install:** the "published build differs" path replaced nothing ([#135](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/135)) ([e5297b3](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e5297b3e6798a424ceb9986d767ebc4b0b47993a))
* **install:** the PATH advice named a file macOS never reads ([#136](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/136)) ([263095c](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/263095c37415b9210a4a250e915972e8ac26d082))
* **install:** the three ways a Windows install could end badly ([#118](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/118)) ([97c2f8f](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/97c2f8fd2fb1fbd4ffe1701a453d513b1795e0f9))
* **install:** upgrade on CONTENT, not on the version string ([#122](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/122)) ([016cf28](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/016cf286813c89c8862cd307221052d8b206c8de))
* **launcher:** stop could not reach a cluster rank's container ([#133](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/133)) ([e8897a1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e8897a1338bffb7f6fbf226551a8edf434ef13bc))
* **launch:** refuse remotely what the CLI only cautions about ([#87](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/87)) ([5e84474](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/5e844740d99dfb5fc66a511f508c6daba5474199))
* **lifecycle:** say "not running" instead of quoting the docker daemon ([#98](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/98)) ([a023034](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/a023034978139bd4c5d82e03a7925308588da337))
* **logs:** find the container `run` actually started ([#142](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/142)) ([9ded867](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/9ded867515fdf9fbf1c6a642de68db4dd60128b1))
* **pair:** a headless agent was reported as no agent at all ([#143](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/143)) ([53d9629](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/53d9629a203d315306a32e2b9574b6c67e78cea3))
* **pair:** sanitise the peer's name before it reaches a terminal ([#86](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/86)) ([e63441d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e63441d7005d2f9bba2220115aefdb837dbc0ae8))
* **peer:** check the pairing code before opening a socket ([#101](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/101)) ([770837b](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/770837b81e8e11ea5cfb1e483e262270cdc5b910))
* **peer:** sanitise the refusal, and check the version both ways ([#89](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/89)) ([54da03d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/54da03d8e346bacb1fcd32c915cb6a544da86ec4))
* **rank:** a rank may only stop a container this fleet launched ([#94](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/94)) ([105ce8e](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/105ce8ead67c13dd666c8f6b69303284b03e5840))
* **rankservice:** the reservation TTL panicked on Windows, breaking main ([#128](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/128)) ([537b01e](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/537b01e1708613a8f848e8519a8ee28b0a6d4a21))
* **recipe:** a recipe may not read the agent's secrets or its egress policy ([#91](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/91)) ([9fe4b3a](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/9fe4b3a63097163f683a8cb18211c2f8a483fe77))
* **recipe:** refuse a field that would be read as an option ([#93](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/93)) ([6487c66](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/6487c669568d279c0e086ee78bdddd57effec4f1))
* **recipe:** suggest the recipe you meant, not the three that sort first ([#96](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/96)) ([e88874d](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/e88874d3b2ffca70537ace311624b8f2f9d54508))
* **registry:** a scope should not cost the operator the suggestion ([#111](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/111)) ([5b6eb5e](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/5b6eb5e7d670d19ad1e145212f3092ea9c4c8644))
* **registry:** a scoped ref cannot walk out of the recipe cache ([#92](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/92)) ([46eccef](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/46eccef8c07965e03544b29c9da057efbc2f6d30))
* **run:** the CLI never checked a `-o` value against its declared bound ([#102](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/102)) ([c3ae320](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/c3ae320362b3833ea1c6ae433c6825450d34b443))
* **run:** the weights pre-flight must not block a launch that brings its own ([#106](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/106)) ([6e046df](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/6e046df1e2d0446f1de042053ec884fd10f6f085))
* **run:** warn about a full disk before the pull, not after ([#116](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/116)) ([df676f6](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/df676f682da9068948722bd7c322e45f1388913c))
* **scrape:** a chunk length that lands mid-character must not panic ([#95](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/95)) ([405d6e0](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/405d6e032bf39c2e1981833825f556803e607dab))
* **secretfile:** canonicalising every write raced Windows file replacement ([#132](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/132)) ([efe1325](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/efe132585c2756a28d04f3e1a3126b1db60ece31))
* **settings:** `port` is the TCP domain, not the IANA registered range ([#105](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/105)) ([d971dd1](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/d971dd1e8da22cec0e83d00640dd0cab7a911527))
* six audit findings, including two regressions from tonight ([#104](https://github.com/Avarok-Cybersecurity/atlas-recipes/issues/104)) ([7e3386f](https://github.com/Avarok-Cybersecurity/atlas-recipes/commit/7e3386fa62de3c866292fa87dbebc132a68ca135))

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
