# Changelog

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
