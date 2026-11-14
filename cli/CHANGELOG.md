# Changelog

## [0.1.1](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0...iohr/v0.1.1) (2026-11-14)


### Features

* **rust:** login primitives in inorbithr behind the login feature (RFC 0231 slice 2) ([#566](https://github.com/inorbithr/sdk/issues/566)) ([454e82c](https://github.com/inorbithr/sdk/commit/454e82c3f36a47eb9e421723e3a3afbdcd40a952))
* **rust:** sign-in flows in inorbithr behind the login feature (RFC 0231 slice 3) ([#569](https://github.com/inorbithr/sdk/issues/569)) ([302cdcf](https://github.com/inorbithr/sdk/commit/302cdcf76a55fac77c970c61e8d40ab859b6cc1c))

## [0.1.0](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.39...iohr/v0.1.0) (2026-11-13)


### Features

* **cli:** auto-resolve switch in the panel's Incidents menu ([#508](https://github.com/inorbithr/sdk/issues/508)) ([8738814](https://github.com/inorbithr/sdk/commit/87388144ef904fa6a38b082f318b08ad1b738761))
* **cli:** claims and evidence in the crawl store, one safe opener, the atlas proof crate ([#518](https://github.com/inorbithr/sdk/issues/518)) ([cce4b1e](https://github.com/inorbithr/sdk/commit/cce4b1ebb7f30155114f5ad12bf1d408cf021075))
* **cli:** compact the handoff note across runs and add iohr handoff ([#480](https://github.com/inorbithr/sdk/issues/480)) ([c1a3f14](https://github.com/inorbithr/sdk/commit/c1a3f1414d9722f78db93ebf6007b66119340e74))
* **cli:** evals and extension templates in every SDK language ([#536](https://github.com/inorbithr/sdk/issues/536)) ([5845f1e](https://github.com/inorbithr/sdk/commit/5845f1efb1ddcef61c75c9bf10a527a789d65d10))
* **cli:** framed chat box for ? and ?? in the panel ([#483](https://github.com/inorbithr/sdk/issues/483)) ([04a6f10](https://github.com/inorbithr/sdk/commit/04a6f10ad83ef143ee62069cb1a99f4d52793c45))
* **cli:** host actions from signed extensions, schema v1 and a build volume with no default ([#469](https://github.com/inorbithr/sdk/issues/469)) ([27e3266](https://github.com/inorbithr/sdk/commit/27e3266a521041ba5f1b788621cfa1b861808fa3))
* **cli:** init asks where to keep project data and the project's shape (PRD 0210) ([#510](https://github.com/inorbithr/sdk/issues/510)) ([5c7b73d](https://github.com/inorbithr/sdk/commit/5c7b73d3169e114e1754f2f8e4077b94fa2afb35))
* **cli:** init templates with an enterprise floor (blank, service, mcp-server, agent, evals, cli, extension) ([#516](https://github.com/inorbithr/sdk/issues/516)) ([738b24d](https://github.com/inorbithr/sdk/commit/738b24dca1e9ca43ff7d2a88de1a26d7ffe19860))
* **cli:** iohr doctor, a local check with an opt-in safe fix, shown under ? and ?? ([#509](https://github.com/inorbithr/sdk/issues/509)) ([344728d](https://github.com/inorbithr/sdk/commit/344728d518f01c3f16a8fbb7e43d84666d101a61))
* **cli:** iohr runner revoke ([#377](https://github.com/inorbithr/sdk/issues/377)) ([04db077](https://github.com/inorbithr/sdk/commit/04db0770fcb568021578f2f60c6d7732c084ecbc))
* **cli:** iohr ui gets the Mac app's welcome, frame, settings and sign-in (RFC 0217) ([#515](https://github.com/inorbithr/sdk/issues/515)) ([fdc21c5](https://github.com/inorbithr/sdk/commit/fdc21c59bba54a4a10cb5d930b46d53f77b5c3e5))
* **cli:** iohr ui serve on loopback (RFC 0217 C1) ([#485](https://github.com/inorbithr/sdk/issues/485)) ([037fd3d](https://github.com/inorbithr/sdk/commit/037fd3d9d1ab42582e6372e8919a004f9c44fc89))
* **cli:** iohr ui serve takes writes and streams; apiSend, apiStream, useLive ([#526](https://github.com/inorbithr/sdk/issues/526)) ([3126983](https://github.com/inorbithr/sdk/commit/3126983a0bac1becb2385488d735d8e5e599afca))
* **cli:** iohr upgrade also upgrades installed extensions ([#412](https://github.com/inorbithr/sdk/issues/412)) ([190062a](https://github.com/inorbithr/sdk/commit/190062abc9459617f008106548955ac4e9364902))
* **cli:** Java, C#, Dart, Swift for service, cli, blank; TypeScript for cli ([#535](https://github.com/inorbithr/sdk/issues/535)) ([8c8ea51](https://github.com/inorbithr/sdk/commit/8c8ea51358c8d8416ba52f86c72947b8d7722843))
* **cli:** mcp-server and agent templates in every SDK language; the picker lists the live languages ([#530](https://github.com/inorbithr/sdk/issues/530)) ([f4f39f8](https://github.com/inorbithr/sdk/commit/f4f39f87214914c58927a00aff74528f7ce287fa))
* **cli:** one ask template for every terminal ask; no escape leaks in cards ([#533](https://github.com/inorbithr/sdk/issues/533)) ([95053e5](https://github.com/inorbithr/sdk/commit/95053e5ffb34a2286b50a83dd4c865803877af1d))
* **cli:** share the panel's terminal to the phone ([#506](https://github.com/inorbithr/sdk/issues/506)) ([dc4925b](https://github.com/inorbithr/sdk/commit/dc4925b75df375823e2e02a6a86f8ef69d9216c2))
* **iohr:** init asks where to keep project data and the project's shape (PRD 0210) ([5c7b73d](https://github.com/inorbithr/sdk/commit/5c7b73d3169e114e1754f2f8e4077b94fa2afb35))


### Bug fixes

* **ci:** bounded apt retry, Go 1.26.11/1.27.4, and the Python service template race ([#542](https://github.com/inorbithr/sdk/issues/542)) ([3193878](https://github.com/inorbithr/sdk/commit/3193878e28beed2a9f0b7212efc84c701e53f134))
* **cli:** ask dock stacks copies when a scroll-region reset slips past the proxy ([#531](https://github.com/inorbithr/sdk/issues/531)) ([7470311](https://github.com/inorbithr/sdk/commit/7470311959464d931fe76bfefd6e955aaf91665e))
* **cli:** ask every init question before Create ([#534](https://github.com/inorbithr/sdk/issues/534)) ([c39a8a3](https://github.com/inorbithr/sdk/commit/c39a8a3801af541a547dc1a7dfe9446c31c1babd))
* **cli:** ask place and crawl notice once, add a custom place and --keep-in, make the plan before the shape item ([#514](https://github.com/inorbithr/sdk/issues/514)) ([6cc98d1](https://github.com/inorbithr/sdk/commit/6cc98d1ddaa7d4aebddf19350c794b5b408b738a))
* **cli:** build cleanly on Windows and retry timing-bound pty tests on macOS ([#545](https://github.com/inorbithr/sdk/issues/545)) ([e7a3731](https://github.com/inorbithr/sdk/commit/e7a3731b15ecc2332bc529e4f1ee1719b1ce5c10))
* **cli:** evals and extension templates build on macOS and C# CI, tests build on Windows ([#555](https://github.com/inorbithr/sdk/issues/555)) ([c432efd](https://github.com/inorbithr/sdk/commit/c432efde9a502c4653077a028be926804d4bd128))
* **cli:** name the chat box hide key ⌥h in every terminal ([#517](https://github.com/inorbithr/sdk/issues/517)) ([1cfe754](https://github.com/inorbithr/sdk/commit/1cfe754dee91a889ed3cc7d97ac2aaa53f9c9422))
* **cli:** one PATH lookup that tries PATHEXT on Windows, so the smoke step starts mvn.cmd ([#561](https://github.com/inorbithr/sdk/issues/561)) ([ae5ed62](https://github.com/inorbithr/sdk/commit/ae5ed626c59ba95c06434f3d010eb0efcffd56e7))
* **cli:** put back what the panel covered when the terminal is resized ([#529](https://github.com/inorbithr/sdk/issues/529)) ([44c4def](https://github.com/inorbithr/sdk/commit/44c4def96ab74a5de928f2c9cdcfc03adc625cc8))
* **cli:** resolve named folders on Windows verbatim paths and kill the ([5bd46f8](https://github.com/inorbithr/sdk/commit/5bd46f8610e8fc1c476d6ee65a05da1e4f363ea3))
* **cli:** Role cards carry the answer, name their Role, and only Roles in lane are asked ([#532](https://github.com/inorbithr/sdk/issues/532)) ([572b73b](https://github.com/inorbithr/sdk/commit/572b73bfa39fcd5a494622332ab18b7f5036358f))
* **cli:** Role cards carry the Role's answer, are named, and only Roles in the question's lane are asked ([572b73b](https://github.com/inorbithr/sdk/commit/572b73bfa39fcd5a494622332ab18b7f5036358f))
* **cli:** share child environment scrubbing between ext run and dev and pin runner git config ([#465](https://github.com/inorbithr/sdk/issues/465)) ([51cf890](https://github.com/inorbithr/sdk/commit/51cf89080851968141562b394ce180474d28869a))
* **cli:** templates and settings test hold on Windows ([#556](https://github.com/inorbithr/sdk/issues/556)) ([db9bc47](https://github.com/inorbithr/sdk/commit/db9bc4789738df7e66e4b8d7d8501827c6cb88e6))
* **cli:** the demo child keeps the Program Files folders on Windows so NuGet restores ([#558](https://github.com/inorbithr/sdk/issues/558)) ([5153d07](https://github.com/inorbithr/sdk/commit/5153d07025a973b370c33268796ad241a87c52e6))
* **cli:** the Down key in the panel menu moves the selection instead of closing it ([#505](https://github.com/inorbithr/sdk/issues/505)) ([9988400](https://github.com/inorbithr/sdk/commit/9988400d0bc335a5adfeb92e06ba98aabe71cf7b))
* **cli:** the java cli template's test compares output with \n line ends on Windows ([#560](https://github.com/inorbithr/sdk/issues/560)) ([05a464f](https://github.com/inorbithr/sdk/commit/05a464f633d8bfc901ccbaf1b0d2996cfc521ce9))
* **cli:** the runner trusts the job's own checkout for mise, so checks and builds run ([#410](https://github.com/inorbithr/sdk/issues/410)) ([c578c77](https://github.com/inorbithr/sdk/commit/c578c77d6ad672efd5dfe713e0c35b57031682d0))
* **cli:** the Windows cli leg passes the PATH test and the Rust extension template ([#562](https://github.com/inorbithr/sdk/issues/562)) ([f1daa11](https://github.com/inorbithr/sdk/commit/f1daa11c96003bb6a932d395c0084c326265c23d))
* **deps:** rustix 1.1.6 for RUSTSEC-2026-0486 ([#537](https://github.com/inorbithr/sdk/issues/537)) ([92464e8](https://github.com/inorbithr/sdk/commit/92464e851c0b1b139db2ab382ca242e53222b982))
* **iohr:** a dead key beside other input is not the panel menu key ([9988400](https://github.com/inorbithr/sdk/commit/9988400d0bc335a5adfeb92e06ba98aabe71cf7b))


### Dependencies

* **rust:** bump the rust group across 4 directories with 5 updates ([#540](https://github.com/inorbithr/sdk/issues/540)) ([3f1951e](https://github.com/inorbithr/sdk/commit/3f1951e8fdafc01a1d2a728ac7c66c7419dafe49))
* **rust:** bump vt100 from 0.16.2 to 0.18.1 in /cli in the rust-major group across 1 directory ([#501](https://github.com/inorbithr/sdk/issues/501)) ([821e06d](https://github.com/inorbithr/sdk/commit/821e06d46081a8163f1201421cff438c6c68a9b5))

## [0.1.0-alpha.39](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.38...iohr/v0.1.0-alpha.39) (2026-11-09)


### ⚠ BREAKING CHANGES

* **cli:** iohr project init now defaults to the coordinated template (a coordination.toml and a Planner plan); pass --template demo for the previous default.

### Features

* **cli:** --role on ask and init applies a Role's text and model ([#441](https://github.com/inorbithr/sdk/issues/441)) ([094d1e5](https://github.com/inorbithr/sdk/commit/094d1e5c52f1ef722d62f0851fe07f56d9d879a9))
* **cli:** ? and ?? read IO.md files up the folder chain, iohr context, init starter ([#475](https://github.com/inorbithr/sdk/issues/475)) ([22c9743](https://github.com/inorbithr/sdk/commit/22c974386e05376595f0bf58be608e809ca2875e))
* **cli:** a local site knowledge base for the init crawl (atlas search, ?/?? sources) ([#477](https://github.com/inorbithr/sdk/issues/477)) ([45a3f25](https://github.com/inorbithr/sdk/commit/45a3f25716a60f199684e09bac4796360d31259d))
* **cli:** answer voice leases on a warm claude session ([#470](https://github.com/inorbithr/sdk/issues/470)) ([0b57609](https://github.com/inorbithr/sdk/commit/0b57609ddc7137afe7f3a9f71901600288ec146c))
* **cli:** API key sign-in for Claude and a claude_auth policy ([#471](https://github.com/inorbithr/sdk/issues/471)) ([b8b8aa9](https://github.com/inorbithr/sdk/commit/b8b8aa9e78dd50356d99f6dd433c86480ae3b042))
* **cli:** approve asks from a Mac with Touch ID (RFC 0113) ([#450](https://github.com/inorbithr/sdk/issues/450)) ([36a4111](https://github.com/inorbithr/sdk/commit/36a4111a2356a8a666a90d1af0e718583da1a428))
* **cli:** ask to read instead of "I can't", next steps as actions ([#444](https://github.com/inorbithr/sdk/issues/444)) ([ecbf7ce](https://github.com/inorbithr/sdk/commit/ecbf7ceda59137be70b6557ff97d06c1fcd785ee))
* **cli:** bring the preview CLI work to main (modes, init sync, ext offer, perm card, roles) ([#440](https://github.com/inorbithr/sdk/issues/440)) ([dddf758](https://github.com/inorbithr/sdk/commit/dddf7583d074c732b6191f76bd2fbfbb137bd94b))
* **cli:** claude_auth and codex_auth take any, plan or api ([#493](https://github.com/inorbithr/sdk/issues/493)) ([8d62c03](https://github.com/inorbithr/sdk/commit/8d62c03415dfb5e8a8020b9ff969926a68d76a33))
* **cli:** edge.caddy-site-call host action adds the call site to the edge Caddy ([#492](https://github.com/inorbithr/sdk/issues/492)) ([6d9df5b](https://github.com/inorbithr/sdk/commit/6d9df5b5349e998d9f7263e96fc4cb007b153883))
* **cli:** every ? and ?? ask is answered by the project's Roles (RFC 0213 S2) ([#454](https://github.com/inorbithr/sdk/issues/454)) ([05b5c26](https://github.com/inorbithr/sdk/commit/05b5c26b69d8e1e2d50da64d6907f09a01e042cc))
* **cli:** generate the terminal themes from the Mac app's THEMES (RFC 0212) ([#457](https://github.com/inorbithr/sdk/issues/457)) ([559b2b1](https://github.com/inorbithr/sdk/commit/559b2b16b1225c86f4a2349fef013077a971c7be))
* **cli:** init crawl rate and a progress bar (RFC 0163) ([#478](https://github.com/inorbithr/sdk/issues/478)) ([8c26d1d](https://github.com/inorbithr/sdk/commit/8c26d1dc3d9a4ee92fa80b4f70df5d7ab2dd8b96))
* **cli:** init defaults to the coordinated template and builds through a Planner plan ([#435](https://github.com/inorbithr/sdk/issues/435)) ([bfb0c6c](https://github.com/inorbithr/sdk/commit/bfb0c6c5638cf9367232e0f510267983635fced3))
* **cli:** iohr ask reads more folders (--read repeats, IOHR_ASK_READ, [ask] read, Add folder) ([#448](https://github.com/inorbithr/sdk/issues/448)) ([e337e0b](https://github.com/inorbithr/sdk/commit/e337e0b46383399a0e07f7d9d956351670596146))
* **cli:** iohr panel, a floating menu over the shell with approvals and workloads ([#447](https://github.com/inorbithr/sdk/issues/447)) ([0465360](https://github.com/inorbithr/sdk/commit/0465360189a59115be05e895f86e74e279975924))
* **cli:** iohr session and iohr stop, a session host for the project's Roles on a durable DuckDB queue ([#449](https://github.com/inorbithr/sdk/issues/449)) ([d40415c](https://github.com/inorbithr/sdk/commit/d40415c90b12b892427e098a729118862e38d7dc))
* **cli:** keep one warm voice session ready for the next call ([#487](https://github.com/inorbithr/sdk/issues/487)) ([ca463f2](https://github.com/inorbithr/sdk/commit/ca463f263a07f6f27a97cd6fb807d19e37f75193))
* **cli:** one grammar for the command line: grouped, coloured help and project template list|show|path ([094d1e5](https://github.com/inorbithr/sdk/commit/094d1e5c52f1ef722d62f0851fe07f56d9d879a9))
* **cli:** one grammar for the command line: grouped, coloured help and project template list|show|path ([9db0456](https://github.com/inorbithr/sdk/commit/9db0456dcec60f3898e4aff777ae5648f61924cd))
* **cli:** one grammar for the command line: grouped, coloured help and project template list|show|path ([dddf758](https://github.com/inorbithr/sdk/commit/dddf7583d074c732b6191f76bd2fbfbb137bd94b))
* **cli:** project init reads your public site and keeps it in atlas and memory ([#455](https://github.com/inorbithr/sdk/issues/455)) ([d8c7827](https://github.com/inorbithr/sdk/commit/d8c7827a37cd7eb39cdff7a6db96e17537f717da))
* **cli:** runner workspace prune, user service install and opt-in host git credentials ([#467](https://github.com/inorbithr/sdk/issues/467)) ([8fd69a6](https://github.com/inorbithr/sdk/commit/8fd69a6015936382b079b3e95cf7da2093146317))
* **cli:** the ? card says what it can see and offers to attach the output of a failed command ([#442](https://github.com/inorbithr/sdk/issues/442)) ([9db0456](https://github.com/inorbithr/sdk/commit/9db0456dcec60f3898e4aff777ae5648f61924cd))
* **cli:** the permission ask is an Io card with the action bar (PRD 0197) ([#421](https://github.com/inorbithr/sdk/issues/421)) ([579c0bb](https://github.com/inorbithr/sdk/commit/579c0bbde9b7845c209661786e112470fba9a85c))


### Bug fixes

* **cli:** ? works in the home folder: trust prompt, then reads and asked commands (PRD 0197) ([#422](https://github.com/inorbithr/sdk/issues/422)) ([1d733f6](https://github.com/inorbithr/sdk/commit/1d733f669e8a428348ddef0254eee5ac13b89bb0))
* **cli:** a chosen ? action runs at once, folders with subfolders offer a recursive read, and the card header is just Io ([#451](https://github.com/inorbithr/sdk/issues/451)) ([b75bff5](https://github.com/inorbithr/sdk/commit/b75bff59b02617c4c8ae9afc2684bf9648dd5a9b))
* **cli:** a re-run after stop starts the session host again ([#482](https://github.com/inorbithr/sdk/issues/482)) ([740b82f](https://github.com/inorbithr/sdk/commit/740b82f57046583677f666972b161086e8a952f9))
* **cli:** bound QuietStore to four calls in flight ([#474](https://github.com/inorbithr/sdk/issues/474)) ([c8092e3](https://github.com/inorbithr/sdk/commit/c8092e353eff29d72890555ad69cff7b0bf006bd))
* **cli:** build on Windows: gate the panel's helpers to unix ([#458](https://github.com/inorbithr/sdk/issues/458)) ([fe799e6](https://github.com/inorbithr/sdk/commit/fe799e6c4abbd9d89d2dadc25a662b378c5212f9))
* **cli:** build on Windows: nearest_256 is used by the panel on every platform ([#495](https://github.com/inorbithr/sdk/issues/495)) ([ea8b61c](https://github.com/inorbithr/sdk/commit/ea8b61c467f81cdb13ed49ff3dfffb9562c2acd7))
* **cli:** build the ext tests on Windows (pty helper is unix only) ([#494](https://github.com/inorbithr/sdk/issues/494)) ([5dbedfb](https://github.com/inorbithr/sdk/commit/5dbedfbd7b46bc663aa15c9bf1bc47209c5d3b5d))
* **cli:** check the credential store before signing in ([#437](https://github.com/inorbithr/sdk/issues/437)) ([ce55a85](https://github.com/inorbithr/sdk/commit/ce55a854bf033d7cea12ab92179178a9b4f4608c))
* **cli:** gate the panel's note below the box to unix ([#476](https://github.com/inorbithr/sdk/issues/476)) ([9a30a65](https://github.com/inorbithr/sdk/commit/9a30a652c846a249b5310bba9500158267ed2400))
* **cli:** init drops the editor-ask line and says a coming extension has nothing to install ([#452](https://github.com/inorbithr/sdk/issues/452)) ([864e46e](https://github.com/inorbithr/sdk/commit/864e46e9d27341827eaf98a8f48990dbd2701087))
* **cli:** init lists what it did not do only under --verbose ([#443](https://github.com/inorbithr/sdk/issues/443)) ([30f7eb0](https://github.com/inorbithr/sdk/commit/30f7eb07ef65557d8882b2921fc56d1a32f68c46))
* **cli:** IO.md labels and runner purge on Windows ([#489](https://github.com/inorbithr/sdk/issues/489)) ([4b1cf8b](https://github.com/inorbithr/sdk/commit/4b1cf8bb890d027623f6fb178a8085fcbf7a8a04))
* **cli:** IO.md tests and file names on Windows and macOS ([#484](https://github.com/inorbithr/sdk/issues/484)) ([8e8db19](https://github.com/inorbithr/sdk/commit/8e8db193046478db443cd4d98980867940122b42))
* **cli:** name Windows paths in a question, and keep / working after a \\?\ prefix ([#464](https://github.com/inorbithr/sdk/issues/464)) ([c7ecf15](https://github.com/inorbithr/sdk/commit/c7ecf15e8e360333c52467ac4f689982a78dd6a9))
* **cli:** one folder-trust prompt, on the Io card (PRD 0197) ([#423](https://github.com/inorbithr/sdk/issues/423)) ([4b7b681](https://github.com/inorbithr/sdk/commit/4b7b6810df0beb630ce7514cf59922342df114d1))
* **cli:** panel keys and restart; the session host gets a runner of its own ([#459](https://github.com/inorbithr/sdk/issues/459)) ([a5a3fbf](https://github.com/inorbithr/sdk/commit/a5a3fbf43ac83a0f1f476bb2bf1dc19ea85bdeb1))
* **cli:** pin release signer identity to main ([#462](https://github.com/inorbithr/sdk/issues/462)) ([c962d99](https://github.com/inorbithr/sdk/commit/c962d99b55aede1f8366b44f18a04c7cf9549887))
* **cli:** Rust reference examples are one main that returns the error ([#439](https://github.com/inorbithr/sdk/issues/439)) ([f279f3f](https://github.com/inorbithr/sdk/commit/f279f3ff08dae13cca360f94c40afd93022e94b1))
* **cli:** sign iohr-preview like the release and ask the Keychain once per run ([#479](https://github.com/inorbithr/sdk/issues/479)) ([111af09](https://github.com/inorbithr/sdk/commit/111af09e06c03306422ec02f53ac004f9873d427))
* **cli:** the ? card's Down menu works after an empty Enter and after the hook is sourced again ([#446](https://github.com/inorbithr/sdk/issues/446)) ([faae596](https://github.com/inorbithr/sdk/commit/faae596d4be455cd8afd489b892c7513096e7bec))
* **cli:** the real Keychain test runs only where CI asks for it ([#472](https://github.com/inorbithr/sdk/issues/472)) ([9b9d886](https://github.com/inorbithr/sdk/commit/9b9d886a00b2918b83c4785ece52f859751cbc43))
* **cli:** two Windows test failures on the alpha.39 release (path separators) ([#502](https://github.com/inorbithr/sdk/issues/502)) ([bc1f453](https://github.com/inorbithr/sdk/commit/bc1f453090ca97d92a4a2d1e2861297fc5360855))
* **cli:** usage errors help: what was missing, real examples, --help and an AI ask offer ([#438](https://github.com/inorbithr/sdk/issues/438)) ([1c5bb69](https://github.com/inorbithr/sdk/commit/1c5bb696d52baa2e6b68cd8a1e55d40f4ce3d255))
* **cli:** write the killed-host test's marker by rename so the parent never reads it empty ([#490](https://github.com/inorbithr/sdk/issues/490)) ([3cfa2ce](https://github.com/inorbithr/sdk/commit/3cfa2ce1e8b9bca31afceb8416b82c4b2cb517b3))


### Documentation

* **cli:** point to the developer-site guides and say what iohr-preview is ([#430](https://github.com/inorbithr/sdk/issues/430)) ([ece935e](https://github.com/inorbithr/sdk/commit/ece935e32f0738db42bd05dc54a83202f668b54f))
* **cli:** split the README into a landing page and topic pages ([#463](https://github.com/inorbithr/sdk/issues/463)) ([b39f4f0](https://github.com/inorbithr/sdk/commit/b39f4f02da90d69df35c73e6bfeabb27f9ec38a9))

## [0.1.0-alpha.38](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.37...iohr/v0.1.0-alpha.38) (2026-11-06)


### ⚠ BREAKING CHANGES

* **rust:** a model or query-parameter struct can no longer be built with a struct literal or `..Default::default()` outside the crate; use `Model::default().with_field(value)` (or `Model::new(..)`). A match on a generated enum needs a `_` arm. Surface cannot be implemented outside the generated surface.
* the RFCs API is the Decisions API. `iohr rfc` is removed with no alias: use `iohr decisions`. The review default reads IOHR_DECISIONS_REVIEWER instead of IOHR_RFC_REVIEWER, and tokens need decisions:read / decisions:write instead of rfc:read / rfc:write.
* **rust:** with the `otel` feature, `ClientBuilder::tracer_provider` and `meter_provider` take opentelemetry 0.33 types; an application on opentelemetry 0.32 upgrades it to 0.33 too.
* request fields are optional in every language (a Rust request is built with ..Default::default(), a Go scalar is a pointer set with inorbit.Ptr); answer fields with presence (messages, optional, oneof) are optional where they used to be typed as always present.

### Features

* a Swift package, its runtime and iohr sdk generate --lang swift ([#175](https://github.com/inorbithr/sdk/issues/175)) ([5ea90cf](https://github.com/inorbithr/sdk/commit/5ea90cf3fc1aeae0fd2e99207ba74a473447b35e))
* **cli:** --account on iohr coordinator commands ([#202](https://github.com/inorbithr/sdk/issues/202)) ([830ffbc](https://github.com/inorbithr/sdk/commit/830ffbc4fcf802b92d8bf84a8b8aed67f11e45ae))
* **cli:** --env and IOHR_ENV target production or local ([#208](https://github.com/inorbithr/sdk/issues/208)) ([3d44e90](https://github.com/inorbithr/sdk/commit/3d44e90345c0dba35d187a33f82ef685a85bce96))
* **cli:** --sign-in io files coordinator jobs as Io, with --via-personality attribution ([#374](https://github.com/inorbithr/sdk/issues/374)) ([8bab38c](https://github.com/inorbithr/sdk/commit/8bab38cfda2a8b8cc1573d8165ef30a5a3aebe10))
* **cli:** a branded sign-in page that says what really happened ([#182](https://github.com/inorbithr/sdk/issues/182)) ([e5e5de4](https://github.com/inorbithr/sdk/commit/e5e5de4255d482bad107864ded6df051e9cd1038))
* **cli:** a guided, drawn project init with menus and spinners (PRD 0088.1) ([#363](https://github.com/inorbithr/sdk/issues/363)) ([148669d](https://github.com/inorbithr/sdk/commit/148669dc592c58d3533be1e7a3f0bf99755d7b3b))
* **cli:** a language-neutral generator core; six languages known ([#41](https://github.com/inorbithr/sdk/issues/41)) ([e6952a0](https://github.com/inorbithr/sdk/commit/e6952a000a7c7193906c16da2f0440918da317e1))
* **cli:** a runner turn logs its usage and cost and leaves user settings out of its context ([#252](https://github.com/inorbithr/sdk/issues/252)) ([97f42d7](https://github.com/inorbithr/sdk/commit/97f42d7ac334c82ad25aa5a006aef73d89bef1be))
* **cli:** add preview.take, roll.task, build.sweep-targets and gh.release-rerun host actions ([#316](https://github.com/inorbithr/sdk/issues/316)) ([e8c294f](https://github.com/inorbithr/sdk/commit/e8c294fba59f8abef777f84c4ffcdc6e033bba1b))
* **cli:** agent.install-checks host action ([#329](https://github.com/inorbithr/sdk/issues/329)) ([fb5ab69](https://github.com/inorbithr/sdk/commit/fb5ab698eeaf823f446f38e020bd05e2a1ec1a6a))
* **cli:** classified spans and review keys in iohr lab check ([#153](https://github.com/inorbithr/sdk/issues/153)) ([a8bacc7](https://github.com/inorbithr/sdk/commit/a8bacc7b35c60a477af4751ed692e3dd8d024949))
* **cli:** cluster target, [panel] and $REPOS for extensions, ext verify on an artifact ([#206](https://github.com/inorbithr/sdk/issues/206)) ([07b2d1c](https://github.com/inorbithr/sdk/commit/07b2d1cb2df660992a58a4e3607eb473dd3ca99b))
* **cli:** disk.prune and helm.runner-scale host actions ([#334](https://github.com/inorbithr/sdk/issues/334)) ([6908058](https://github.com/inorbithr/sdk/commit/6908058d20e2f487e11032b116134b71c8524043))
* **cli:** enrol the run key through an ask approved on the phone ([#295](https://github.com/inorbithr/sdk/issues/295)) ([4be4686](https://github.com/inorbithr/sdk/commit/4be4686431c0750a52eef342c56c5a39fbba5c1b))
* **cli:** execute approved pinned run asks (k8s.delete-failed-pods) ([#285](https://github.com/inorbithr/sdk/issues/285)) ([efe6e72](https://github.com/inorbithr/sdk/commit/efe6e72eaf60c1ae4532814c938f81b3c98220a1))
* **cli:** extensions declare privileges, confirmed before install ([#151](https://github.com/inorbithr/sdk/issues/151)) ([fdf0784](https://github.com/inorbithr/sdk/commit/fdf07841bb537a3af13921028c7ab541bb5aff9a))
* **cli:** headless iohr ask and iohr shell hook zsh for the ? prompt ([#375](https://github.com/inorbithr/sdk/issues/375)) ([b6c2943](https://github.com/inorbithr/sdk/commit/b6c2943e173a4a3f793b71669a1395b7a9b79591))
* **cli:** install.sh and install.ps1, checked against the release ([#18](https://github.com/inorbithr/sdk/issues/18)) ([7cff577](https://github.com/inorbithr/sdk/commit/7cff57755e379325b9ca04232ea6767e8a52b6bd))
* **cli:** iohr api --all walks every page of a list into one answer ([#95](https://github.com/inorbithr/sdk/issues/95)) ([ea86b94](https://github.com/inorbithr/sdk/commit/ea86b94fc45b30f2833c1ad850efb28e20759a20))
* **cli:** iohr ask runs your own agent in this folder (PRD 0088.1) ([#361](https://github.com/inorbithr/sdk/issues/361)) ([bfdc6f7](https://github.com/inorbithr/sdk/commit/bfdc6f76f69549724b9134f833a69ad731f7a84e))
* **cli:** iohr browser install, the browser extension from its store ([#178](https://github.com/inorbithr/sdk/issues/178)) ([94e74c4](https://github.com/inorbithr/sdk/commit/94e74c4c3f378ef2569a25f4cf55babcb545eb83))
* **cli:** iohr connectors and iohr connections (RFC 0044) ([#105](https://github.com/inorbithr/sdk/issues/105)) ([a964223](https://github.com/inorbithr/sdk/commit/a96422376ddcd970c4b18900d1b0349639d497ba))
* **cli:** iohr coordinator asks create --kind ([#204](https://github.com/inorbithr/sdk/issues/204)) ([05abcbd](https://github.com/inorbithr/sdk/commit/05abcbdad2790c2be695901691b3bacf8eb69231))
* **cli:** iohr coordinator asks create --kind, and a KIND column on asks list ([05abcbd](https://github.com/inorbithr/sdk/commit/05abcbdad2790c2be695901691b3bacf8eb69231))
* **cli:** iohr coordinator brief &lt;job&gt; ([#203](https://github.com/inorbithr/sdk/issues/203)) ([91b4387](https://github.com/inorbithr/sdk/commit/91b4387b087814c4fd7774d76927ae1c01947146))
* **cli:** iohr coordinator brief &lt;job&gt; prints a job's capped brief ([91b4387](https://github.com/inorbithr/sdk/commit/91b4387b087814c4fd7774d76927ae1c01947146))
* **cli:** iohr coordinator jobs list|create and asks list ([#201](https://github.com/inorbithr/sdk/issues/201)) ([7c12ae4](https://github.com/inorbithr/sdk/commit/7c12ae4037450a732cb70558363f21c14a1f1504))
* **cli:** iohr ext (RFC 0028) and iohr domains (RFC 0030) ([#40](https://github.com/inorbithr/sdk/issues/40)) ([dbae5c8](https://github.com/inorbithr/sdk/commit/dbae5c8cb211c7decb652df910f90d476e35951d))
* **cli:** iohr ext init, dev and lint, the local extension loop (PRD 0084 slice 1) ([#191](https://github.com/inorbithr/sdk/issues/191)) ([9083c23](https://github.com/inorbithr/sdk/commit/9083c23036e34e9bdc897b8839f331246366998a))
* **cli:** iohr ext packages and iohr packages list every package, private and local too ([#197](https://github.com/inorbithr/sdk/issues/197)) ([df68793](https://github.com/inorbithr/sdk/commit/df687936866877643ca57423813a586bd880d7b9))
* **cli:** iohr ext search, show and install PUBLISHER/NAME from the catalogue ([#168](https://github.com/inorbithr/sdk/issues/168)) ([79a4661](https://github.com/inorbithr/sdk/commit/79a466144af7d32ac256642ea2f5822fb7907402))
* **cli:** iohr ext service, and install offers an extension's system service ([#186](https://github.com/inorbithr/sdk/issues/186)) ([e6783ea](https://github.com/inorbithr/sdk/commit/e6783ea7a8a5ada504dc104d1abea9bd7199284a))
* **cli:** iohr lab check, the lab document checks on your own repository ([#53](https://github.com/inorbithr/sdk/issues/53)) ([9cb02be](https://github.com/inorbithr/sdk/commit/9cb02be784c5f45b9096228defd192cf00755e12))
* **cli:** iohr login in a browser or with a device code ([#15](https://github.com/inorbithr/sdk/issues/15)) ([5387abc](https://github.com/inorbithr/sdk/commit/5387abc8d5b838c12b92729485fc375e35152918))
* **cli:** iohr project create, templates, a configured default and the coordinated template ([#359](https://github.com/inorbithr/sdk/issues/359)) ([54f0d0a](https://github.com/inorbithr/sdk/commit/54f0d0ae5239b3b936a3e687306c409fb1fd4260))
* **cli:** iohr project init, a project of your own set up with one command (PRD 0088) ([#193](https://github.com/inorbithr/sdk/issues/193)) ([b7ed488](https://github.com/inorbithr/sdk/commit/b7ed48869f544bbba2fa358251f7cf6fd34853de))
* **cli:** iohr rfc names PRDs and ADRs by kind (platform RFC 0081) ([#181](https://github.com/inorbithr/sdk/issues/181)) ([c30c8d4](https://github.com/inorbithr/sdk/commit/c30c8d42a617efc7155825c65259c6e153a8fdf0))
* **cli:** iohr rfc, RFCs in the RFCs product ([#167](https://github.com/inorbithr/sdk/issues/167)) ([5da90ef](https://github.com/inorbithr/sdk/commit/5da90ef2322b7c37726fe24f2b1b22e79b679f20))
* **cli:** iohr runner can sign in as Io (client credentials) ([#342](https://github.com/inorbithr/sdk/issues/342)) ([9965993](https://github.com/inorbithr/sdk/commit/9965993f81c82e75ca2b7876c43d1eec36668e1f))
* **cli:** iohr runner gate-report runs the check and sends the signed gate report ([#242](https://github.com/inorbithr/sdk/issues/242)) ([0646127](https://github.com/inorbithr/sdk/commit/06461271f76e0959f3779471307d041d0df78505))
* **cli:** iohr runner register, run and status ([#190](https://github.com/inorbithr/sdk/issues/190)) ([7405c0b](https://github.com/inorbithr/sdk/commit/7405c0bd5613861b0abb806f9adbc11130b1bb86))
* **cli:** iohr runner run-key --show prints this machine's run key and enrolment ([#300](https://github.com/inorbithr/sdk/issues/300)) ([9f4848a](https://github.com/inorbithr/sdk/commit/9f4848a716e6c0f99e8fde6d280c73a0d4d27056))
* **cli:** iohr sdk add installs the SDK with the project's package manager ([#144](https://github.com/inorbithr/sdk/issues/144)) ([9473dfe](https://github.com/inorbithr/sdk/commit/9473dfeea44bf1bc9a8e0ad2b91f3d23a8d59699))
* **cli:** iohr sdk config prints the SDKs' effective configuration ([#110](https://github.com/inorbithr/sdk/issues/110)) ([e4a9d9c](https://github.com/inorbithr/sdk/commit/e4a9d9cef7652fc179f384074b3e42cb801d23a5))
* **cli:** iohr sdk config resolves through the Rust SDK's loader, and the Rust target emits the idempotency-key mark and the path template ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **cli:** iohr sdk examples, one compile-checked snippet per operation and language ([#107](https://github.com/inorbithr/sdk/issues/107)) ([f0b871e](https://github.com/inorbithr/sdk/commit/f0b871e14112179b4caed8285e387ad015972af6))
* **cli:** iohr signs in with an API token, keeps profiles and calls the API ([#14](https://github.com/inorbithr/sdk/issues/14)) ([6c3189c](https://github.com/inorbithr/sdk/commit/6c3189c62df5ebfeb612be21a244f5802509872c))
* **cli:** iohr tunnel, a loopback pipe into an environment's Envoy edge ([#212](https://github.com/inorbithr/sdk/issues/212)) ([ba33bdf](https://github.com/inorbithr/sdk/commit/ba33bdf4f7e1ba6868acb64000ce0dbeee9f9ac4))
* **cli:** iohr upgrade, brew trust in the install docs, cli:tap-local ([#213](https://github.com/inorbithr/sdk/issues/213)) ([aaae388](https://github.com/inorbithr/sdk/commit/aaae388c8b8dd3ab0896316ad1024771425a1573))
* **cli:** iterators over paged lists in TypeScript, Python, Rust, Java and C# ([#66](https://github.com/inorbithr/sdk/issues/66)) ([3a42fd6](https://github.com/inorbithr/sdk/commit/3a42fd63fbfc855a916e8694f0ff5c0fe4c7bab5))
* **cli:** labs.reattribute-document host action ([#378](https://github.com/inorbithr/sdk/issues/378)) ([ffe4924](https://github.com/inorbithr/sdk/commit/ffe49240056b79bf99b2f7f01efdf8986096aaa5))
* **cli:** let iohr runner sign in as Io with client credentials ([9965993](https://github.com/inorbithr/sdk/commit/9965993f81c82e75ca2b7876c43d1eec36668e1f))
* **cli:** one grammar for the command line: grouped, coloured help and project template list|show|path ([53e0c8f](https://github.com/inorbithr/sdk/commit/53e0c8f51d940b04d3d92570d06f8aba8d439a00))
* **cli:** one grammar for the command line: grouped, coloured help and project template list|show|path ([34f6a0f](https://github.com/inorbithr/sdk/commit/34f6a0f6cff812989c5bae8f93d2b95cf9d23214))
* **cli:** project templates as data, your agent in init, and a real dry run ([#365](https://github.com/inorbithr/sdk/issues/365)) ([34f6a0f](https://github.com/inorbithr/sdk/commit/34f6a0f6cff812989c5bae8f93d2b95cf9d23214))
* **cli:** run five more host actions through one argv runner ([#306](https://github.com/inorbithr/sdk/issues/306)) ([06d7d3e](https://github.com/inorbithr/sdk/commit/06d7d3ee02376b6478429a45894075f4d2261469))
* **cli:** run up to --capacity turns at once in the runner loop ([#240](https://github.com/inorbithr/sdk/issues/240)) ([87b6702](https://github.com/inorbithr/sdk/commit/87b67021b9371d325e1477075d19944dac6ad2a9))
* **cli:** runner capacity and kinds from config.toml, declared on every heartbeat ([#244](https://github.com/inorbithr/sdk/issues/244)) ([2a01d11](https://github.com/inorbithr/sdk/commit/2a01d113e1be2a0d84cf5388fd7606578d86002d))
* **cli:** runner holds its heartbeat so a Talk turn starts at once (RFC 0193) ([#281](https://github.com/inorbithr/sdk/issues/281)) ([61a8c39](https://github.com/inorbithr/sdk/commit/61a8c396a87f9d467e1531796321aaf7fe77cdf2))
* **cli:** runner pushes with its lease's own GitHub App token ([#267](https://github.com/inorbithr/sdk/issues/267)) ([d9b7e48](https://github.com/inorbithr/sdk/commit/d9b7e48d3f9b41919344cfd9ddc2dc2f73faf5d9))
* **cli:** runner register sends the holder subject for Io's runners ([#356](https://github.com/inorbithr/sdk/issues/356)) ([7aabe3f](https://github.com/inorbithr/sdk/commit/7aabe3f31d12466a15d4d0f4cfaa1c41ff98568a))
* **cli:** sdk generate from profiles, sdk check with iohr.lock, profile account ([#36](https://github.com/inorbithr/sdk/issues/36)) ([748f30a](https://github.com/inorbithr/sdk/commit/748f30ab65499dd8b9d31a3e909a75bb6dfbd1ae))
* **cli:** share config.toml with the SDKs, add iohr auth token ([#109](https://github.com/inorbithr/sdk/issues/109)) ([16cb8ef](https://github.com/inorbithr/sdk/commit/16cb8ef26303385fedda8ce3fcbfa4235b2b2606))
* **cli:** sign in on the account you chose or made your default ([#299](https://github.com/inorbithr/sdk/issues/299)) ([5a4bc6d](https://github.com/inorbithr/sdk/commit/5a4bc6d323e0184807d1a840905105c19ab28ba5))
* **cli:** the ? card asks back with an action bar, permission asks, asks in the terminal, init demo A to Z ([#409](https://github.com/inorbithr/sdk/issues/409)) ([53e0c8f](https://github.com/inorbithr/sdk/commit/53e0c8f51d940b04d3d92570d06f8aba8d439a00))
* **cli:** the C# target emits the path template and the idempotency-key mark ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **cli:** the generator marks operations that take Idempotency-Key ([#111](https://github.com/inorbithr/sdk/issues/111)) ([1390526](https://github.com/inorbithr/sdk/commit/1390526e579e4c213cebca9cff75ab1ed0a3a0fc))
* **cli:** the generator with the Rust target; the crate's public surface is generated ([#35](https://github.com/inorbithr/sdk/issues/35)) ([d4e77cf](https://github.com/inorbithr/sdk/commit/d4e77cffd5d7a20a77a484fdd56a59fbcac1dd4d))
* **cli:** the runner builds every job of a repository into one shared target directory ([#349](https://github.com/inorbithr/sdk/issues/349)) ([c8dbf69](https://github.com/inorbithr/sdk/commit/c8dbf690a20d22149829e918d9f1c286b951d992))
* **cli:** the runner owns the job's checkout and opens its pull request ([#260](https://github.com/inorbithr/sdk/issues/260)) ([ed95f1e](https://github.com/inorbithr/sdk/commit/ed95f1e7bce40a214ca667661e532bc191d6848d))
* **cli:** the runner reports its cores, memory, disks and load on every beat ([#305](https://github.com/inorbithr/sdk/issues/305)) ([fc1c390](https://github.com/inorbithr/sdk/commit/fc1c39085dbae82fbeec4c7bea22d0570d5abda6))
* **cli:** the runner upgrades itself and restarts ([#309](https://github.com/inorbithr/sdk/issues/309)) ([8f8b69a](https://github.com/inorbithr/sdk/commit/8f8b69a525062ca7e33f9499530bee3c1c42664d))
* **cli:** the runner writes the Critique's record_critique, never the model ([#218](https://github.com/inorbithr/sdk/issues/218)) ([f74a779](https://github.com/inorbithr/sdk/commit/f74a7798cd528df2d6cb2eacc683da6ea22a6c8f))
* **conformance:** replay vectors prints the vectors as JSON for drivers without a YAML reader ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **csharp:** calls have a total timeout of 120 s by default (TotalTimeout), every attempt and wait included ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** Client.Load with one precedence (code, environment, the iohr config file, defaults), Describe() and ConfigException problems ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** proxy, CA bundle, mTLS, pinning and connect timeout on SocketsHttpHandler, or a caller's HttpClient or handler ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** the C# runtime and target ([#58](https://github.com/inorbithr/sdk/issues/58)) ([bc1e7bb](https://github.com/inorbithr/sdk/commit/bc1e7bbcb0927247e31d754a92020a153489995b))
* **csharp:** the credential chain (env, token and key secret files, the iohr login) with CachedToken, TokenFile, CliToken, ChainedCredential and DefaultCredential ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** the named middleware pipeline with its built-ins, Middleware.FromHandler, ILogger logging, ActivitySource and Meter, rate limits and the retry budget ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* Decisions API, the contract synced to 136 operations, upgrade_required ([#189](https://github.com/inorbithr/sdk/issues/189)) ([91f998c](https://github.com/inorbithr/sdk/commit/91f998c9b53bb25587469e32155e893bcefe765c))
* **go:** calls have a total timeout of 120 s by default (WithTotalTimeout), every attempt and wait included ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** inorbit.Load with one precedence (code, environment, the iohr config file, defaults), Describe and ConfigError problems ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** iterators over paged lists, slog logging, Example functions, virtual-clock tests ([#60](https://github.com/inorbithr/sdk/issues/60)) ([069a9cb](https://github.com/inorbithr/sdk/commit/069a9cb0bc516be82d2f290457c1455b752f669b))
* **go:** OpenTelemetry spans and metrics in the optional module github.com/inorbithr/sdk/go/otel ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** proxy, CA bundle, mTLS, pinning and connect timeout through the SDK's own http.Transport, or a caller's client or RoundTripper ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** the credential chain (env, token and key secret files, the iohr login) with CachedToken, TokenFile, CliToken, ChainedCredential and DefaultCredential ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** the Go runtime and target ([#56](https://github.com/inorbithr/sdk/issues/56)) ([8bbebcb](https://github.com/inorbithr/sdk/commit/8bbebcb4870fa86fbd45e55b0fee6bd1ddf871e4))
* **go:** the named middleware pipeline over http.RoundTripper with its built-ins, log/slog logging, rate limits and the retry budget ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **java:** calls have a total timeout of 120 s by default, and the attempt timeout covers the body ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** Client.load with one precedence (code, environment, the iohr config file, defaults), describe() and ConfigException problems ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** proxy, CA bundle, mTLS, pinning and connect timeout through the SDK's own ProxySelector and SSLContext ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** the credential chain (env, token and key secret files, the iohr login) with TokenFile, CliToken, CachedToken, ChainedCredential and DefaultCredential ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** the Java runtime and the Java target ([#59](https://github.com/inorbithr/sdk/issues/59)) ([325f2f9](https://github.com/inorbithr/sdk/commit/325f2f953bfa0e985bf45ff0203ae7dd7452ca78))
* **java:** the named middleware pipeline with its built-ins, System.Logger logging, optional OpenTelemetry, rate limits and the retry budget ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **py:** calls have a 120 s total deadline by default (total_timeout) ([3c8075d](https://github.com/inorbithr/sdk/commit/3c8075d922cd8341dd80815636d7936d56815cdd))
* **py:** load, the credential chain, the middleware pipeline, transport settings, logging and OpenTelemetry (M6) ([3c8075d](https://github.com/inorbithr/sdk/commit/3c8075d922cd8341dd80815636d7936d56815cdd))
* **py:** the Python runtime and target ([#57](https://github.com/inorbithr/sdk/issues/57)) ([e823d2e](https://github.com/inorbithr/sdk/commit/e823d2e9f4e91c29ae14d7ff0efa71be416f9a15))
* **py:** writes that take Idempotency-Key are retried, with one key per call ([3c8075d](https://github.com/inorbithr/sdk/commit/3c8075d922cd8341dd80815636d7936d56815cdd))
* **rust:** calls have a total timeout of 120 s by default (total_timeout), and a Retry-After longer than retry_after_max ends the call ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** Client::load with one precedence (code, environment, the iohr config file, defaults), describe() and ConfigError problems ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** models and query parameters are non_exhaustive, built with setters; Surface is sealed ([#180](https://github.com/inorbithr/sdk/issues/180)) ([918625a](https://github.com/inorbithr/sdk/commit/918625a9b2a4e02f2f58499cc5546cd7c759e9b0))
* **rust:** proxy, CA bundle, mTLS, pinning and connect timeout through the SDK's own rules, or a caller's reqwest::Client ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** the credential chain (env, token and key secret files, the iohr login) with TokenFile, CliToken, CachedToken, ChainedCredential and DefaultCredential ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** the named middleware pipeline with its built-ins, tracing logging, OpenTelemetry behind the otel feature, rate limits and the retry budget ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **spec:** the contract synced with the platform; unprocessable in every runtime ([#65](https://github.com/inorbithr/sdk/issues/65)) ([69aad11](https://github.com/inorbithr/sdk/commit/69aad110b1e77c4c49241846581959537dfcf15f))
* streams in all six languages, over server-sent events or the /v1/ws socket (M5) ([#96](https://github.com/inorbithr/sdk/issues/96)) ([7ff73fe](https://github.com/inorbithr/sdk/commit/7ff73fe986f4706299d188ef9a4d5273dc0ca7ee))
* the contract states what the sync patched in; requests leave out unset fields ([#85](https://github.com/inorbithr/sdk/issues/85)) ([9eff311](https://github.com/inorbithr/sdk/commit/9eff311ab88bee9873a777b90197c16b471809e2))
* **ts:** calls have a total timeout of 120 s by default (totalTimeout), every attempt and wait included ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** Client.load with one precedence (code, environment, the iohr config file, defaults), describe() and ConfigError problems ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** proxy, CA bundle, mTLS, pinning and connect timeout through a node:https transport, or a caller's fetch or dispatcher ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** the credential chain (env, token and key secret files, the iohr login) with CachedToken, TokenFile, CliToken, ChainedCredential and DefaultCredential ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** the named middleware pipeline with its built-ins, logging, OpenTelemetry, rate limits and the retry budget ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** the TypeScript runtime and target ([#46](https://github.com/inorbithr/sdk/issues/46)) ([3a2b2b0](https://github.com/inorbithr/sdk/commit/3a2b2b03f3325bac7a048cd642d6ac5c93f495b3))
* **ts:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))


### Bug fixes

* **ci:** exclude release-please changelogs from typos, fix Windows lints ([#234](https://github.com/inorbithr/sdk/issues/234)) ([fbdf42e](https://github.com/inorbithr/sdk/commit/fbdf42efeb428c28375cd0aced9bc75a957019b4))
* **ci:** Go 1.26.10 and 1.27.3 for GO-2026-6809; gate test holds on Windows ([#250](https://github.com/inorbithr/sdk/issues/250)) ([34ab38e](https://github.com/inorbithr/sdk/commit/34ab38e10094d64ffdb9e05440fa0e5adfa02aad))
* **cli:** a new platform error code no longer stops sdk generate ([#61](https://github.com/inorbithr/sdk/issues/61)) ([b6689ef](https://github.com/inorbithr/sdk/commit/b6689ef6611663164e03d113b7684b9ead0ec7e2))
* **cli:** build the kubeconfig path in a test the way the platform spells an absolute one ([#294](https://github.com/inorbithr/sdk/issues/294)) ([5704ef7](https://github.com/inorbithr/sdk/commit/5704ef7a095071388b77a9c1da7eab7a118c07c7))
* **cli:** bump sigstore-verify to 0.15 to drop block-buffer 0.10.4 (GHSA-qwgh-2vcv-g2f7) ([#282](https://github.com/inorbithr/sdk/issues/282)) ([b042f01](https://github.com/inorbithr/sdk/commit/b042f019ddacf430e020a0c532132e91098f09ae))
* **cli:** check the sysctl path as POSIX so the test passes on Windows ([#315](https://github.com/inorbithr/sdk/issues/315)) ([7c4e5ee](https://github.com/inorbithr/sdk/commit/7c4e5ee4003784404ab6a1cafa82346ff9d4d097))
* **cli:** clippy 1.99 assert_is_empty in the upgrade test ([#216](https://github.com/inorbithr/sdk/issues/216)) ([04ed677](https://github.com/inorbithr/sdk/commit/04ed67726a99c32366eadbd988a0a39e4f52d0d0))
* **cli:** gate tail drops the partial first line after the bounded drain ([#354](https://github.com/inorbithr/sdk/issues/354)) ([508f7ce](https://github.com/inorbithr/sdk/commit/508f7ceb023afefa06d944f7a2e2c95e9b817da4))
* **cli:** gate unix-only permission and card helpers so Windows clippy passes ([#419](https://github.com/inorbithr/sdk/issues/419)) ([cdb992f](https://github.com/inorbithr/sdk/commit/cdb992f8a1c2d2fcd0837e9a7b30e9022281d07c))
* **cli:** Go reference examples are one main with log.Fatal ([#348](https://github.com/inorbithr/sdk/issues/348)) ([9f0dc41](https://github.com/inorbithr/sdk/commit/9f0dc411fe571063f4744d6f5a7a3b1d287a0639))
* **cli:** identity hash is stable across token refresh ([#357](https://github.com/inorbithr/sdk/issues/357)) ([603cae8](https://github.com/inorbithr/sdk/commit/603cae83372d3dfed868fa5f1f85f5e4eb09e1af))
* **cli:** idiomatic reference examples in every language, with Go Close and Python with ([#332](https://github.com/inorbithr/sdk/issues/332)) ([f2eeb74](https://github.com/inorbithr/sdk/commit/f2eeb74d38dba4071df1449f674f10b25ed49231))
* **cli:** iohr login --scope, so the runner's scopes can be asked for ([#232](https://github.com/inorbithr/sdk/issues/232)) ([a7d03c8](https://github.com/inorbithr/sdk/commit/a7d03c860ac76d82fc8a7df97460122d882ef256))
* **cli:** iohr login --scope, so the runner's scopes can be asked for (RFC 0120) ([a7d03c8](https://github.com/inorbithr/sdk/commit/a7d03c860ac76d82fc8a7df97460122d882ef256))
* **cli:** iohr runner acts on the profile's account, not the token's personal one ([#236](https://github.com/inorbithr/sdk/issues/236)) ([ffc5663](https://github.com/inorbithr/sdk/commit/ffc5663f35a23f937245af90a062882ba2a4955a))
* **cli:** IOHR_TOKEN wins over the default profile, as the README and ADR 0009 say ([16cb8ef](https://github.com/inorbithr/sdk/commit/16cb8ef26303385fedda8ce3fcbfa4235b2b2606))
* **cli:** keep a redacted tail of a failing gate check ([#328](https://github.com/inorbithr/sdk/issues/328)) ([dd26b41](https://github.com/inorbithr/sdk/commit/dd26b419dca3e3cacc2e5eef44dfee49d573501b))
* **cli:** keep the Windows private-file check clippy clean ([#293](https://github.com/inorbithr/sdk/issues/293)) ([49f385d](https://github.com/inorbithr/sdk/commit/49f385d62a1f45665186af63e4698e0ac9116f73))
* **cli:** make the Windows CLI job green again ([#397](https://github.com/inorbithr/sdk/issues/397)) ([28db3cf](https://github.com/inorbithr/sdk/commit/28db3cf4a03191aee80211111c3fc18a63f2754b))
* **cli:** merge main into the job branch before the runner ships ([#261](https://github.com/inorbithr/sdk/issues/261)) ([df9a9c6](https://github.com/inorbithr/sdk/commit/df9a9c6b54c2d44e07d9ef90dbbf98f0f58ad550))
* **cli:** name the command, not iohr.exe, in what init writes on Windows ([#420](https://github.com/inorbithr/sdk/issues/420)) ([3cf8780](https://github.com/inorbithr/sdk/commit/3cf87809516bcefa4874f1902dafabf2e0e4fa41))
* **cli:** never title a runner PR from the status line; derive a Conventional Commit title and repair cut ones ([#283](https://github.com/inorbithr/sdk/issues/283)) ([3bf04f4](https://github.com/inorbithr/sdk/commit/3bf04f4df08989f18c725c7b457adfc6a2b6a6dd))
* **cli:** one runner per key; pin the runner by id in config ([#389](https://github.com/inorbithr/sdk/issues/389)) ([bbbfda4](https://github.com/inorbithr/sdk/commit/bbbfda4d98c19bd4fc5d0e9df20d591051f31894))
* **cli:** read macOS sysctl by absolute path so the Mac runner reports memory and load ([#314](https://github.com/inorbithr/sdk/issues/314)) ([ab2923c](https://github.com/inorbithr/sdk/commit/ab2923c91c336967ab16736a683a417f4764aa5e))
* **cli:** restart a runner whose binary is already the newest release ([#399](https://github.com/inorbithr/sdk/issues/399)) ([7ed11aa](https://github.com/inorbithr/sdk/commit/7ed11aa3b51332e9108ae135815f409fb4fd278f))
* **cli:** runner gate check fetches main with the lease's token ([#302](https://github.com/inorbithr/sdk/issues/302)) ([3ad0655](https://github.com/inorbithr/sdk/commit/3ad06556e668f68b13eb0c6873a13c401bbaaa0c))
* **cli:** say why a credential-store call waits (macOS Keychain prompt) ([#392](https://github.com/inorbithr/sdk/issues/392)) ([c986353](https://github.com/inorbithr/sdk/commit/c986353a4d8ae4962a3a270eba7154d496c39e47))
* **cli:** sdk config's path rules and vector tests hold on Windows ([#120](https://github.com/inorbithr/sdk/issues/120)) ([c4fdefd](https://github.com/inorbithr/sdk/commit/c4fdefd2912ce8ac709805f55f7e7de82589d9e0))
* **cli:** set the Io secret file mode only on Unix in tests ([#382](https://github.com/inorbithr/sdk/issues/382)) ([4cacc98](https://github.com/inorbithr/sdk/commit/4cacc989efaeb958ec7536500e40b8d7817962b5))
* **cli:** settings tests use a platform-absolute path ([#317](https://github.com/inorbithr/sdk/issues/317)) ([636d33e](https://github.com/inorbithr/sdk/commit/636d33e4a1ad19db8ab86195ebe8a208d6f5ced6))
* **cli:** tests never open a real browser ([#390](https://github.com/inorbithr/sdk/issues/390)) ([7e8007e](https://github.com/inorbithr/sdk/commit/7e8007ee5072770c880c55bfa27b498113812b96))
* **cli:** the client-policy step sends the file body over stdin, not argv ([#370](https://github.com/inorbithr/sdk/issues/370)) ([64e43e1](https://github.com/inorbithr/sdk/commit/64e43e1e7f3dbb42618f55254220fd96c9fa784d))
* **cli:** the ext tests build on Windows ([#51](https://github.com/inorbithr/sdk/issues/51)) ([1ee3f8b](https://github.com/inorbithr/sdk/commit/1ee3f8b004c863def0540132b6e81d4d49cef056))
* **cli:** the extension tests pass on slow and Windows runners ([#258](https://github.com/inorbithr/sdk/issues/258)) ([ed6113a](https://github.com/inorbithr/sdk/commit/ed6113ac09a40f7bcd48f7adc579626f5f5e3e17))
* **cli:** the gate tail test's import builds on Windows ([#331](https://github.com/inorbithr/sdk/issues/331)) ([28a8cda](https://github.com/inorbithr/sdk/commit/28a8cda0d6a69e7623f7133ff19fdf62d705c875))
* **cli:** the Homebrew formula's description passes brew audit ([#29](https://github.com/inorbithr/sdk/issues/29)) ([7535a35](https://github.com/inorbithr/sdk/commit/7535a35602182773be9b08e8353d4c55ba6248f9))
* **cli:** the host probe test runs only where /proc or sysctl exist ([#308](https://github.com/inorbithr/sdk/issues/308)) ([5e4590f](https://github.com/inorbithr/sdk/commit/5e4590ff5375a5f34453258e5ad0a093d183c965))
* **cli:** the last Unix-only import in the ext tests moves with its helper ([#52](https://github.com/inorbithr/sdk/issues/52)) ([87c3cfd](https://github.com/inorbithr/sdk/commit/87c3cfdb3838eb115b5da6d714414e47c456c75d))
* **cli:** the release scripts live in cli/release ([#27](https://github.com/inorbithr/sdk/issues/27)) ([eef71a8](https://github.com/inorbithr/sdk/commit/eef71a87b8f8bf395e50279339c9f9190507b244))
* **cli:** the runner re-reads its sign-in after a 401 and exits when it stays refused ([#400](https://github.com/inorbithr/sdk/issues/400)) ([ebec2d7](https://github.com/inorbithr/sdk/commit/ebec2d75e8f83e3deac573a2c8f0f262284c7cce))
* **cli:** the runner's run loop sends the gate report for a finished job turn ([#254](https://github.com/inorbithr/sdk/issues/254)) ([3346863](https://github.com/inorbithr/sdk/commit/3346863b9de6f6393f9cf00ea057a87f09c0ea08))
* **cli:** the scrubbed-environment gate test uses cmd syntax on Windows ([#257](https://github.com/inorbithr/sdk/issues/257)) ([5360ef5](https://github.com/inorbithr/sdk/commit/5360ef5e13122f68a2ecf3d6e7bed0bf348ef462))
* **cli:** the upgrade tests reach their mock server on Windows ([#259](https://github.com/inorbithr/sdk/issues/259)) ([d76a915](https://github.com/inorbithr/sdk/commit/d76a91535bb17a350d0d1b69e500ffc7f12abfec))
* **cli:** upgrade skips a release whose files are not uploaded yet ([#303](https://github.com/inorbithr/sdk/issues/303)) ([b0b7beb](https://github.com/inorbithr/sdk/commit/b0b7beb26464ef7c8cbe471880415b7c152b86f8))
* **cli:** upgrades keep a stable macOS signature so the keychain grant holds ([#391](https://github.com/inorbithr/sdk/issues/391)) ([e4a9710](https://github.com/inorbithr/sdk/commit/e4a9710096fa558abf1b10439c2bcbc441694ccb))
* **conformance:** the replay server hands back action options and rewrite ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **deps:** bump rustls-webpki and webpki-root-certs for RUSTSEC-2026-0382 and 0383 ([#245](https://github.com/inorbithr/sdk/issues/245)) ([22cc5c6](https://github.com/inorbithr/sdk/commit/22cc5c6402899919bfc87f4745b171770462d1ab))
* **deps:** ctutils 0.4.5 in every Cargo lock and opentelemetry-api 1.45.1 in the Python lock so the advisory audits pass again ([#256](https://github.com/inorbithr/sdk/issues/256)) ([91446a5](https://github.com/inorbithr/sdk/commit/91446a5fea6fd4769d6d7fe4bd2c55de6b7b5316))
* extension tokens for a person, and a wait that outlasts the channel ([#205](https://github.com/inorbithr/sdk/issues/205)) ([b57dbef](https://github.com/inorbithr/sdk/commit/b57dbef00126617a197ccdbd378b2170696cac2d))
* **py:** the attempt timeout covers the body; the socket sends an iohr- request id and uses the proxy; the token exchange sends the SDK's user agent; from_env normalises profile names ([3c8075d](https://github.com/inorbithr/sdk/commit/3c8075d922cd8341dd80815636d7936d56815cdd))
* **repo:** keep ids and account data out of test messages, verify TLS in the pin test ([#346](https://github.com/inorbithr/sdk/issues/346)) ([518f21e](https://github.com/inorbithr/sdk/commit/518f21ead46bba023fadd577b80537db23283f9c))
* **runner:** settings tests use a platform-absolute path ([636d33e](https://github.com/inorbithr/sdk/commit/636d33e4a1ad19db8ab86195ebe8a208d6f5ced6))
* **rust:** the socket task ends when the client is dropped ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **security:** keep ids and account data out of test messages; verify TLS in the pin test ([518f21e](https://github.com/inorbithr/sdk/commit/518f21ead46bba023fadd577b80537db23283f9c))


### Documentation

* bring every README to what is released and on main ([#155](https://github.com/inorbithr/sdk/issues/155)) ([637daeb](https://github.com/inorbithr/sdk/commit/637daeb6a1b40d014773b227e218502b17a40806))
* **docs:** READMEs match today's SDKs and CLI ([#323](https://github.com/inorbithr/sdk/issues/323)) ([52970cb](https://github.com/inorbithr/sdk/commit/52970cb8a0f0484e1312b543c0fc17e8a17e1712))
* **sdk:** READMEs match today's SDKs and CLI ([52970cb](https://github.com/inorbithr/sdk/commit/52970cb8a0f0484e1312b543c0fc17e8a17e1712))


### Dependencies

* **cli:** bump yanked async-executor 1.14.0 to 1.14.1 ([#345](https://github.com/inorbithr/sdk/issues/345)) ([9dbd2ba](https://github.com/inorbithr/sdk/commit/9dbd2bac1d2269f36f1e4d11a18f4d4952c1b809))
* **rust:** bump the rust group across 4 directories with 14 updates ([#238](https://github.com/inorbithr/sdk/issues/238)) ([d2c7787](https://github.com/inorbithr/sdk/commit/d2c778777c522e32641c7afb4f7b6ee1c7465b51))
* **rust:** bump the rust-major group across 2 directories with 1 update ([#241](https://github.com/inorbithr/sdk/issues/241)) ([1179884](https://github.com/inorbithr/sdk/commit/1179884ddea88e8ab03ba3eedc37954dcd8ba645))
* **rust:** opentelemetry 0.33, schemars held at 0.8 for typify ([#170](https://github.com/inorbithr/sdk/issues/170)) ([dd98b90](https://github.com/inorbithr/sdk/commit/dd98b9036907ab0647a20554fb65f6f4926c1605))
* **rust:** tokio-tungstenite 0.30, pkcs8 0.11 and fresh lockfiles ([#135](https://github.com/inorbithr/sdk/issues/135)) ([d13b761](https://github.com/inorbithr/sdk/commit/d13b761ad7b96924c1d14bea08ecd3cd3517aace))

## [0.1.0-alpha.37](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.36...iohr/v0.1.0-alpha.37) (2026-11-06)


### Bug fixes

* **cli:** tests never open a real browser ([#390](https://github.com/inorbithr/sdk/issues/390)) ([7e8007e](https://github.com/inorbithr/sdk/commit/7e8007ee5072770c880c55bfa27b498113812b96))
* **cli:** the client-policy step sends the file body over stdin, not argv ([#370](https://github.com/inorbithr/sdk/issues/370)) ([64e43e1](https://github.com/inorbithr/sdk/commit/64e43e1e7f3dbb42618f55254220fd96c9fa784d))
* **cli:** the runner re-reads its sign-in after a 401 and exits when it stays refused ([#400](https://github.com/inorbithr/sdk/issues/400)) ([ebec2d7](https://github.com/inorbithr/sdk/commit/ebec2d75e8f83e3deac573a2c8f0f262284c7cce))

## [0.1.0-alpha.36](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.35...iohr/v0.1.0-alpha.36) (2026-11-05)


### Features

* **cli:** headless iohr ask and iohr shell hook zsh for the ? prompt ([#375](https://github.com/inorbithr/sdk/issues/375)) ([b6c2943](https://github.com/inorbithr/sdk/commit/b6c2943e173a4a3f793b71669a1395b7a9b79591))
* **cli:** one grammar for the command line: grouped, coloured help and project template list|show|path ([34f6a0f](https://github.com/inorbithr/sdk/commit/34f6a0f6cff812989c5bae8f93d2b95cf9d23214))
* **cli:** project templates as data, your agent in init, and a real dry run ([#365](https://github.com/inorbithr/sdk/issues/365)) ([34f6a0f](https://github.com/inorbithr/sdk/commit/34f6a0f6cff812989c5bae8f93d2b95cf9d23214))


### Bug fixes

* **cli:** make the Windows CLI job green again ([#397](https://github.com/inorbithr/sdk/issues/397)) ([28db3cf](https://github.com/inorbithr/sdk/commit/28db3cf4a03191aee80211111c3fc18a63f2754b))
* **cli:** one runner per key; pin the runner by id in config ([#389](https://github.com/inorbithr/sdk/issues/389)) ([bbbfda4](https://github.com/inorbithr/sdk/commit/bbbfda4d98c19bd4fc5d0e9df20d591051f31894))
* **cli:** restart a runner whose binary is already the newest release ([#399](https://github.com/inorbithr/sdk/issues/399)) ([7ed11aa](https://github.com/inorbithr/sdk/commit/7ed11aa3b51332e9108ae135815f409fb4fd278f))
* **cli:** say why a credential-store call waits (macOS Keychain prompt) ([#392](https://github.com/inorbithr/sdk/issues/392)) ([c986353](https://github.com/inorbithr/sdk/commit/c986353a4d8ae4962a3a270eba7154d496c39e47))
* **cli:** upgrades keep a stable macOS signature so the keychain grant holds ([#391](https://github.com/inorbithr/sdk/issues/391)) ([e4a9710](https://github.com/inorbithr/sdk/commit/e4a9710096fa558abf1b10439c2bcbc441694ccb))

## [0.1.0-alpha.35](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.34...iohr/v0.1.0-alpha.35) (2026-11-05)


### Features

* **cli:** --sign-in io files coordinator jobs as Io, with --via-personality attribution ([#374](https://github.com/inorbithr/sdk/issues/374)) ([8bab38c](https://github.com/inorbithr/sdk/commit/8bab38cfda2a8b8cc1573d8165ef30a5a3aebe10))
* **cli:** labs.reattribute-document host action ([#378](https://github.com/inorbithr/sdk/issues/378)) ([ffe4924](https://github.com/inorbithr/sdk/commit/ffe49240056b79bf99b2f7f01efdf8986096aaa5))


### Bug fixes

* **cli:** set the Io secret file mode only on Unix in tests ([#382](https://github.com/inorbithr/sdk/issues/382)) ([4cacc98](https://github.com/inorbithr/sdk/commit/4cacc989efaeb958ec7536500e40b8d7817962b5))

## [0.1.0-alpha.34](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.33...iohr/v0.1.0-alpha.34) (2026-11-05)


### Bug fixes

* **cli:** identity hash is stable across token refresh ([#357](https://github.com/inorbithr/sdk/issues/357)) ([603cae8](https://github.com/inorbithr/sdk/commit/603cae83372d3dfed868fa5f1f85f5e4eb09e1af))

## [0.1.0-alpha.33](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.32...iohr/v0.1.0-alpha.33) (2026-11-05)


### Features

* **cli:** a guided, drawn project init with menus and spinners (PRD 0088.1) ([#363](https://github.com/inorbithr/sdk/issues/363)) ([148669d](https://github.com/inorbithr/sdk/commit/148669dc592c58d3533be1e7a3f0bf99755d7b3b))
* **cli:** iohr ask runs your own agent in this folder (PRD 0088.1) ([#361](https://github.com/inorbithr/sdk/issues/361)) ([bfdc6f7](https://github.com/inorbithr/sdk/commit/bfdc6f76f69549724b9134f833a69ad731f7a84e))
* **cli:** iohr project create, templates, a configured default and the coordinated template ([#359](https://github.com/inorbithr/sdk/issues/359)) ([54f0d0a](https://github.com/inorbithr/sdk/commit/54f0d0ae5239b3b936a3e687306c409fb1fd4260))

## [0.1.0-alpha.32](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.31...iohr/v0.1.0-alpha.32) (2026-11-04)


### Features

* **cli:** runner holds its heartbeat so a Talk turn starts at once (RFC 0193) ([#281](https://github.com/inorbithr/sdk/issues/281)) ([61a8c39](https://github.com/inorbithr/sdk/commit/61a8c396a87f9d467e1531796321aaf7fe77cdf2))
* **cli:** runner register sends the holder subject for Io's runners ([#356](https://github.com/inorbithr/sdk/issues/356)) ([7aabe3f](https://github.com/inorbithr/sdk/commit/7aabe3f31d12466a15d4d0f4cfaa1c41ff98568a))
* **cli:** the runner builds every job of a repository into one shared target directory ([#349](https://github.com/inorbithr/sdk/issues/349)) ([c8dbf69](https://github.com/inorbithr/sdk/commit/c8dbf690a20d22149829e918d9f1c286b951d992))
* **cli:** the runner writes the Critique's record_critique, never the model ([#218](https://github.com/inorbithr/sdk/issues/218)) ([f74a779](https://github.com/inorbithr/sdk/commit/f74a7798cd528df2d6cb2eacc683da6ea22a6c8f))


### Bug fixes

* **cli:** gate tail drops the partial first line after the bounded drain ([#354](https://github.com/inorbithr/sdk/issues/354)) ([508f7ce](https://github.com/inorbithr/sdk/commit/508f7ceb023afefa06d944f7a2e2c95e9b817da4))
* **cli:** Go reference examples are one main with log.Fatal ([#348](https://github.com/inorbithr/sdk/issues/348)) ([9f0dc41](https://github.com/inorbithr/sdk/commit/9f0dc411fe571063f4744d6f5a7a3b1d287a0639))
* **repo:** keep ids and account data out of test messages, verify TLS in the pin test ([#346](https://github.com/inorbithr/sdk/issues/346)) ([518f21e](https://github.com/inorbithr/sdk/commit/518f21ead46bba023fadd577b80537db23283f9c))
* **security:** keep ids and account data out of test messages; verify TLS in the pin test ([518f21e](https://github.com/inorbithr/sdk/commit/518f21ead46bba023fadd577b80537db23283f9c))

## [0.1.0-alpha.31](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.30...iohr/v0.1.0-alpha.31) (2026-11-03)


### Features

* **cli:** disk.prune and helm.runner-scale host actions ([#334](https://github.com/inorbithr/sdk/issues/334)) ([6908058](https://github.com/inorbithr/sdk/commit/6908058d20e2f487e11032b116134b71c8524043))
* **cli:** iohr runner can sign in as Io (client credentials) ([#342](https://github.com/inorbithr/sdk/issues/342)) ([9965993](https://github.com/inorbithr/sdk/commit/9965993f81c82e75ca2b7876c43d1eec36668e1f))
* **cli:** let iohr runner sign in as Io with client credentials ([9965993](https://github.com/inorbithr/sdk/commit/9965993f81c82e75ca2b7876c43d1eec36668e1f))


### Dependencies

* **cli:** bump yanked async-executor 1.14.0 to 1.14.1 ([#345](https://github.com/inorbithr/sdk/issues/345)) ([9dbd2ba](https://github.com/inorbithr/sdk/commit/9dbd2bac1d2269f36f1e4d11a18f4d4952c1b809))

## [0.1.0-alpha.30](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.29...iohr/v0.1.0-alpha.30) (2026-10-31)


### Features

* **cli:** agent.install-checks host action ([#329](https://github.com/inorbithr/sdk/issues/329)) ([fb5ab69](https://github.com/inorbithr/sdk/commit/fb5ab698eeaf823f446f38e020bd05e2a1ec1a6a))
* **cli:** iohr runner run-key --show prints this machine's run key and enrolment ([#300](https://github.com/inorbithr/sdk/issues/300)) ([9f4848a](https://github.com/inorbithr/sdk/commit/9f4848a716e6c0f99e8fde6d280c73a0d4d27056))


### Bug fixes

* **cli:** idiomatic reference examples in every language, with Go Close and Python with ([#332](https://github.com/inorbithr/sdk/issues/332)) ([f2eeb74](https://github.com/inorbithr/sdk/commit/f2eeb74d38dba4071df1449f674f10b25ed49231))
* **cli:** keep a redacted tail of a failing gate check ([#328](https://github.com/inorbithr/sdk/issues/328)) ([dd26b41](https://github.com/inorbithr/sdk/commit/dd26b419dca3e3cacc2e5eef44dfee49d573501b))
* **cli:** the gate tail test's import builds on Windows ([#331](https://github.com/inorbithr/sdk/issues/331)) ([28a8cda](https://github.com/inorbithr/sdk/commit/28a8cda0d6a69e7623f7133ff19fdf62d705c875))


### Documentation

* **docs:** READMEs match today's SDKs and CLI ([#323](https://github.com/inorbithr/sdk/issues/323)) ([52970cb](https://github.com/inorbithr/sdk/commit/52970cb8a0f0484e1312b543c0fc17e8a17e1712))
* **sdk:** READMEs match today's SDKs and CLI ([52970cb](https://github.com/inorbithr/sdk/commit/52970cb8a0f0484e1312b543c0fc17e8a17e1712))

## [0.1.0-alpha.29](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.28...iohr/v0.1.0-alpha.29) (2026-10-29)


### Features

* **cli:** add preview.take, roll.task, build.sweep-targets and gh.release-rerun host actions ([#316](https://github.com/inorbithr/sdk/issues/316)) ([e8c294f](https://github.com/inorbithr/sdk/commit/e8c294fba59f8abef777f84c4ffcdc6e033bba1b))
* **cli:** run five more host actions through one argv runner ([#306](https://github.com/inorbithr/sdk/issues/306)) ([06d7d3e](https://github.com/inorbithr/sdk/commit/06d7d3ee02376b6478429a45894075f4d2261469))
* **cli:** the runner upgrades itself and restarts ([#309](https://github.com/inorbithr/sdk/issues/309)) ([8f8b69a](https://github.com/inorbithr/sdk/commit/8f8b69a525062ca7e33f9499530bee3c1c42664d))


### Bug fixes

* **cli:** check the sysctl path as POSIX so the test passes on Windows ([#315](https://github.com/inorbithr/sdk/issues/315)) ([7c4e5ee](https://github.com/inorbithr/sdk/commit/7c4e5ee4003784404ab6a1cafa82346ff9d4d097))
* **cli:** read macOS sysctl by absolute path so the Mac runner reports memory and load ([#314](https://github.com/inorbithr/sdk/issues/314)) ([ab2923c](https://github.com/inorbithr/sdk/commit/ab2923c91c336967ab16736a683a417f4764aa5e))
* **cli:** settings tests use a platform-absolute path ([#317](https://github.com/inorbithr/sdk/issues/317)) ([636d33e](https://github.com/inorbithr/sdk/commit/636d33e4a1ad19db8ab86195ebe8a208d6f5ced6))
* **runner:** settings tests use a platform-absolute path ([636d33e](https://github.com/inorbithr/sdk/commit/636d33e4a1ad19db8ab86195ebe8a208d6f5ced6))

## [0.1.0-alpha.28](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.27...iohr/v0.1.0-alpha.28) (2026-10-28)


### Features

* **cli:** the runner reports its cores, memory, disks and load on every beat ([#305](https://github.com/inorbithr/sdk/issues/305)) ([fc1c390](https://github.com/inorbithr/sdk/commit/fc1c39085dbae82fbeec4c7bea22d0570d5abda6))


### Bug fixes

* **cli:** the host probe test runs only where /proc or sysctl exist ([#308](https://github.com/inorbithr/sdk/issues/308)) ([5e4590f](https://github.com/inorbithr/sdk/commit/5e4590ff5375a5f34453258e5ad0a093d183c965))
* **cli:** upgrade skips a release whose files are not uploaded yet ([#303](https://github.com/inorbithr/sdk/issues/303)) ([b0b7beb](https://github.com/inorbithr/sdk/commit/b0b7beb26464ef7c8cbe471880415b7c152b86f8))

## [0.1.0-alpha.27](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.26...iohr/v0.1.0-alpha.27) (2026-10-28)


### Features

* **cli:** enrol the run key through an ask approved on the phone ([#295](https://github.com/inorbithr/sdk/issues/295)) ([4be4686](https://github.com/inorbithr/sdk/commit/4be4686431c0750a52eef342c56c5a39fbba5c1b))
* **cli:** sign in on the account you chose or made your default ([#299](https://github.com/inorbithr/sdk/issues/299)) ([5a4bc6d](https://github.com/inorbithr/sdk/commit/5a4bc6d323e0184807d1a840905105c19ab28ba5))


### Bug fixes

* **cli:** never title a runner PR from the status line; derive a Conventional Commit title and repair cut ones ([#283](https://github.com/inorbithr/sdk/issues/283)) ([3bf04f4](https://github.com/inorbithr/sdk/commit/3bf04f4df08989f18c725c7b457adfc6a2b6a6dd))
* **cli:** runner gate check fetches main with the lease's token ([#302](https://github.com/inorbithr/sdk/issues/302)) ([3ad0655](https://github.com/inorbithr/sdk/commit/3ad06556e668f68b13eb0c6873a13c401bbaaa0c))

## [0.1.0-alpha.26](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.25...iohr/v0.1.0-alpha.26) (2026-10-27)


### Features

* **cli:** execute approved pinned run asks (k8s.delete-failed-pods) ([#285](https://github.com/inorbithr/sdk/issues/285)) ([efe6e72](https://github.com/inorbithr/sdk/commit/efe6e72eaf60c1ae4532814c938f81b3c98220a1))


### Bug fixes

* **cli:** build the kubeconfig path in a test the way the platform spells an absolute one ([#294](https://github.com/inorbithr/sdk/issues/294)) ([5704ef7](https://github.com/inorbithr/sdk/commit/5704ef7a095071388b77a9c1da7eab7a118c07c7))
* **cli:** bump sigstore-verify to 0.15 to drop block-buffer 0.10.4 (GHSA-qwgh-2vcv-g2f7) ([#282](https://github.com/inorbithr/sdk/issues/282)) ([b042f01](https://github.com/inorbithr/sdk/commit/b042f019ddacf430e020a0c532132e91098f09ae))
* **cli:** keep the Windows private-file check clippy clean ([#293](https://github.com/inorbithr/sdk/issues/293)) ([49f385d](https://github.com/inorbithr/sdk/commit/49f385d62a1f45665186af63e4698e0ac9116f73))

## [0.1.0-alpha.25](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.24...iohr/v0.1.0-alpha.25) (2026-10-24)


### ⚠ BREAKING CHANGES

* **rust:** a model or query-parameter struct can no longer be built with a struct literal or `..Default::default()` outside the crate; use `Model::default().with_field(value)` (or `Model::new(..)`). A match on a generated enum needs a `_` arm. Surface cannot be implemented outside the generated surface.

### Features

* **cli:** runner pushes with its lease's own GitHub App token ([#267](https://github.com/inorbithr/sdk/issues/267)) ([d9b7e48](https://github.com/inorbithr/sdk/commit/d9b7e48d3f9b41919344cfd9ddc2dc2f73faf5d9))
* **rust:** models and query parameters are non_exhaustive, built with setters; Surface is sealed ([#180](https://github.com/inorbithr/sdk/issues/180)) ([918625a](https://github.com/inorbithr/sdk/commit/918625a9b2a4e02f2f58499cc5546cd7c759e9b0))


### Dependencies

* **rust:** bump the rust-major group across 2 directories with 1 update ([#241](https://github.com/inorbithr/sdk/issues/241)) ([1179884](https://github.com/inorbithr/sdk/commit/1179884ddea88e8ab03ba3eedc37954dcd8ba645))

## [0.1.0-alpha.24](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.23...iohr/v0.1.0-alpha.24) (2026-10-24)


### Bug fixes

* **cli:** merge main into the job branch before the runner ships ([#261](https://github.com/inorbithr/sdk/issues/261)) ([df9a9c6](https://github.com/inorbithr/sdk/commit/df9a9c6b54c2d44e07d9ef90dbbf98f0f58ad550))

## [0.1.0-alpha.23](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.22...iohr/v0.1.0-alpha.23) (2026-10-24)


### Features

* **cli:** a runner turn logs its usage and cost and leaves user settings out of its context ([#252](https://github.com/inorbithr/sdk/issues/252)) ([97f42d7](https://github.com/inorbithr/sdk/commit/97f42d7ac334c82ad25aa5a006aef73d89bef1be))
* **cli:** the runner owns the job's checkout and opens its pull request ([#260](https://github.com/inorbithr/sdk/issues/260)) ([ed95f1e](https://github.com/inorbithr/sdk/commit/ed95f1e7bce40a214ca667661e532bc191d6848d))


### Bug fixes

* **ci:** Go 1.26.10 and 1.27.3 for GO-2026-6809; gate test holds on Windows ([#250](https://github.com/inorbithr/sdk/issues/250)) ([34ab38e](https://github.com/inorbithr/sdk/commit/34ab38e10094d64ffdb9e05440fa0e5adfa02aad))
* **cli:** the extension tests pass on slow and Windows runners ([#258](https://github.com/inorbithr/sdk/issues/258)) ([ed6113a](https://github.com/inorbithr/sdk/commit/ed6113ac09a40f7bcd48f7adc579626f5f5e3e17))
* **cli:** the runner's run loop sends the gate report for a finished job turn ([#254](https://github.com/inorbithr/sdk/issues/254)) ([3346863](https://github.com/inorbithr/sdk/commit/3346863b9de6f6393f9cf00ea057a87f09c0ea08))
* **cli:** the scrubbed-environment gate test uses cmd syntax on Windows ([#257](https://github.com/inorbithr/sdk/issues/257)) ([5360ef5](https://github.com/inorbithr/sdk/commit/5360ef5e13122f68a2ecf3d6e7bed0bf348ef462))
* **cli:** the upgrade tests reach their mock server on Windows ([#259](https://github.com/inorbithr/sdk/issues/259)) ([d76a915](https://github.com/inorbithr/sdk/commit/d76a91535bb17a350d0d1b69e500ffc7f12abfec))
* **deps:** ctutils 0.4.5 in every Cargo lock and opentelemetry-api 1.45.1 in the Python lock so the advisory audits pass again ([#256](https://github.com/inorbithr/sdk/issues/256)) ([91446a5](https://github.com/inorbithr/sdk/commit/91446a5fea6fd4769d6d7fe4bd2c55de6b7b5316))

## [0.1.0-alpha.22](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.21...iohr/v0.1.0-alpha.22) (2026-10-21)


### Features

* **cli:** runner capacity and kinds from config.toml, declared on every heartbeat ([#244](https://github.com/inorbithr/sdk/issues/244)) ([2a01d11](https://github.com/inorbithr/sdk/commit/2a01d113e1be2a0d84cf5388fd7606578d86002d))


### Bug fixes

* **deps:** bump rustls-webpki and webpki-root-certs for RUSTSEC-2026-0382 and 0383 ([#245](https://github.com/inorbithr/sdk/issues/245)) ([22cc5c6](https://github.com/inorbithr/sdk/commit/22cc5c6402899919bfc87f4745b171770462d1ab))

## [0.1.0-alpha.21](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.20...iohr/v0.1.0-alpha.21) (2026-10-20)


### Features

* **cli:** iohr runner gate-report runs the check and sends the signed gate report ([#242](https://github.com/inorbithr/sdk/issues/242)) ([0646127](https://github.com/inorbithr/sdk/commit/06461271f76e0959f3779471307d041d0df78505))


### Dependencies

* **rust:** bump the rust group across 4 directories with 14 updates ([#238](https://github.com/inorbithr/sdk/issues/238)) ([d2c7787](https://github.com/inorbithr/sdk/commit/d2c778777c522e32641c7afb4f7b6ee1c7465b51))

## [0.1.0-alpha.20](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.19...iohr/v0.1.0-alpha.20) (2026-10-20)


### Features

* **cli:** run up to --capacity turns at once in the runner loop ([#240](https://github.com/inorbithr/sdk/issues/240)) ([87b6702](https://github.com/inorbithr/sdk/commit/87b67021b9371d325e1477075d19944dac6ad2a9))


### Bug fixes

* **ci:** exclude release-please changelogs from typos, fix Windows lints ([#234](https://github.com/inorbithr/sdk/issues/234)) ([fbdf42e](https://github.com/inorbithr/sdk/commit/fbdf42efeb428c28375cd0aced9bc75a957019b4))

## [0.1.0-alpha.19](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.18...iohr/v0.1.0-alpha.19) (2026-10-19)


### Bug fixes

* **cli:** iohr runner acts on the profile's account, not the token's personal one ([#236](https://github.com/inorbithr/sdk/issues/236)) ([ffc5663](https://github.com/inorbithr/sdk/commit/ffc5663f35a23f937245af90a062882ba2a4955a))

## [0.1.0-alpha.18](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.17...iohr/v0.1.0-alpha.18) (2026-10-19)


### Bug fixes

* **cli:** iohr login --scope, so the runner's scopes can be asked for ([#232](https://github.com/inorbithr/sdk/issues/232)) ([a7d03c8](https://github.com/inorbithr/sdk/commit/a7d03c860ac76d82fc8a7df97460122d882ef256))
* **cli:** iohr login --scope, so the runner's scopes can be asked for (RFC 0120) ([a7d03c8](https://github.com/inorbithr/sdk/commit/a7d03c860ac76d82fc8a7df97460122d882ef256))

## [0.1.0-alpha.17](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.16...iohr/v0.1.0-alpha.17) (2026-10-19)


### Features

* **cli:** --account on iohr coordinator commands ([#202](https://github.com/inorbithr/sdk/issues/202)) ([830ffbc](https://github.com/inorbithr/sdk/commit/830ffbc4fcf802b92d8bf84a8b8aed67f11e45ae))
* **cli:** iohr coordinator asks create --kind ([#204](https://github.com/inorbithr/sdk/issues/204)) ([05abcbd](https://github.com/inorbithr/sdk/commit/05abcbdad2790c2be695901691b3bacf8eb69231))
* **cli:** iohr coordinator asks create --kind, and a KIND column on asks list ([05abcbd](https://github.com/inorbithr/sdk/commit/05abcbdad2790c2be695901691b3bacf8eb69231))
* **cli:** iohr coordinator brief &lt;job&gt; ([#203](https://github.com/inorbithr/sdk/issues/203)) ([91b4387](https://github.com/inorbithr/sdk/commit/91b4387b087814c4fd7774d76927ae1c01947146))
* **cli:** iohr coordinator brief &lt;job&gt; prints a job's capped brief ([91b4387](https://github.com/inorbithr/sdk/commit/91b4387b087814c4fd7774d76927ae1c01947146))
* **cli:** iohr runner register, run and status ([#190](https://github.com/inorbithr/sdk/issues/190)) ([7405c0b](https://github.com/inorbithr/sdk/commit/7405c0bd5613861b0abb806f9adbc11130b1bb86))


### Bug fixes

* **cli:** clippy 1.99 assert_is_empty in the upgrade test ([#216](https://github.com/inorbithr/sdk/issues/216)) ([04ed677](https://github.com/inorbithr/sdk/commit/04ed67726a99c32366eadbd988a0a39e4f52d0d0))

## [0.1.0-alpha.16](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.15...iohr/v0.1.0-alpha.16) (2026-10-16)


### Features

* **cli:** --env and IOHR_ENV target production or local ([#208](https://github.com/inorbithr/sdk/issues/208)) ([3d44e90](https://github.com/inorbithr/sdk/commit/3d44e90345c0dba35d187a33f82ef685a85bce96))
* **cli:** iohr coordinator jobs list|create and asks list ([#201](https://github.com/inorbithr/sdk/issues/201)) ([7c12ae4](https://github.com/inorbithr/sdk/commit/7c12ae4037450a732cb70558363f21c14a1f1504))
* **cli:** iohr ext packages and iohr packages list every package, private and local too ([#197](https://github.com/inorbithr/sdk/issues/197)) ([df68793](https://github.com/inorbithr/sdk/commit/df687936866877643ca57423813a586bd880d7b9))
* **cli:** iohr project init, a project of your own set up with one command (PRD 0088) ([#193](https://github.com/inorbithr/sdk/issues/193)) ([b7ed488](https://github.com/inorbithr/sdk/commit/b7ed48869f544bbba2fa358251f7cf6fd34853de))
* **cli:** iohr tunnel, a loopback pipe into an environment's Envoy edge ([#212](https://github.com/inorbithr/sdk/issues/212)) ([ba33bdf](https://github.com/inorbithr/sdk/commit/ba33bdf4f7e1ba6868acb64000ce0dbeee9f9ac4))
* **cli:** iohr upgrade, brew trust in the install docs, cli:tap-local ([#213](https://github.com/inorbithr/sdk/issues/213)) ([aaae388](https://github.com/inorbithr/sdk/commit/aaae388c8b8dd3ab0896316ad1024771425a1573))


### Bug fixes

* extension tokens for a person, and a wait that outlasts the channel ([#205](https://github.com/inorbithr/sdk/issues/205)) ([b57dbef](https://github.com/inorbithr/sdk/commit/b57dbef00126617a197ccdbd378b2170696cac2d))

## [0.1.0-alpha.15](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.14...iohr/v0.1.0-alpha.15) (2026-10-12)


### ⚠ BREAKING CHANGES

* the RFCs API is the Decisions API. `iohr rfc` is removed with no alias: use `iohr decisions`. The review default reads IOHR_DECISIONS_REVIEWER instead of IOHR_RFC_REVIEWER, and tokens need decisions:read / decisions:write instead of rfc:read / rfc:write.

### Features

* **cli:** iohr ext init, dev and lint, the local extension loop (PRD 0084 slice 1) ([#191](https://github.com/inorbithr/sdk/issues/191)) ([9083c23](https://github.com/inorbithr/sdk/commit/9083c23036e34e9bdc897b8839f331246366998a))
* Decisions API, the contract synced to 136 operations, upgrade_required ([#189](https://github.com/inorbithr/sdk/issues/189)) ([91f998c](https://github.com/inorbithr/sdk/commit/91f998c9b53bb25587469e32155e893bcefe765c))

## [0.1.0-alpha.14](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.13...iohr/v0.1.0-alpha.14) (2026-10-09)


### Features

* **cli:** iohr ext service, and install offers an extension's system service ([#186](https://github.com/inorbithr/sdk/issues/186)) ([e6783ea](https://github.com/inorbithr/sdk/commit/e6783ea7a8a5ada504dc104d1abea9bd7199284a))

## [0.1.0-alpha.13](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.12...iohr/v0.1.0-alpha.13) (2026-10-08)


### ⚠ BREAKING CHANGES

* **rust:** with the `otel` feature, `ClientBuilder::tracer_provider` and `meter_provider` take opentelemetry 0.33 types; an application on opentelemetry 0.32 upgrades it to 0.33 too.

### Features

* **cli:** a branded sign-in page that says what really happened ([#182](https://github.com/inorbithr/sdk/issues/182)) ([e5e5de4](https://github.com/inorbithr/sdk/commit/e5e5de4255d482bad107864ded6df051e9cd1038))
* **cli:** iohr browser install, the browser extension from its store ([#178](https://github.com/inorbithr/sdk/issues/178)) ([94e74c4](https://github.com/inorbithr/sdk/commit/94e74c4c3f378ef2569a25f4cf55babcb545eb83))
* **cli:** iohr rfc names PRDs and ADRs by kind (platform RFC 0081) ([#181](https://github.com/inorbithr/sdk/issues/181)) ([c30c8d4](https://github.com/inorbithr/sdk/commit/c30c8d42a617efc7155825c65259c6e153a8fdf0))


### Dependencies

* **rust:** opentelemetry 0.33, schemars held at 0.8 for typify ([#170](https://github.com/inorbithr/sdk/issues/170)) ([dd98b90](https://github.com/inorbithr/sdk/commit/dd98b9036907ab0647a20554fb65f6f4926c1605))

## [0.1.0-alpha.12](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.11...iohr/v0.1.0-alpha.12) (2026-10-07)


### Features

* **cli:** iohr ext search, show and install PUBLISHER/NAME from the catalogue ([#168](https://github.com/inorbithr/sdk/issues/168)) ([79a4661](https://github.com/inorbithr/sdk/commit/79a466144af7d32ac256642ea2f5822fb7907402))
* **cli:** iohr rfc, RFCs in the RFCs product ([#167](https://github.com/inorbithr/sdk/issues/167)) ([5da90ef](https://github.com/inorbithr/sdk/commit/5da90ef2322b7c37726fe24f2b1b22e79b679f20))

## [0.1.0-alpha.11](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.10...iohr/v0.1.0-alpha.11) (2026-10-06)


### Features

* **cli:** classified spans and review keys in iohr lab check ([#153](https://github.com/inorbithr/sdk/issues/153)) ([a8bacc7](https://github.com/inorbithr/sdk/commit/a8bacc7b35c60a477af4751ed692e3dd8d024949))
* **cli:** extensions declare privileges, confirmed before install ([#151](https://github.com/inorbithr/sdk/issues/151)) ([fdf0784](https://github.com/inorbithr/sdk/commit/fdf07841bb537a3af13921028c7ab541bb5aff9a))


### Documentation

* bring every README to what is released and on main ([#155](https://github.com/inorbithr/sdk/issues/155)) ([637daeb](https://github.com/inorbithr/sdk/commit/637daeb6a1b40d014773b227e218502b17a40806))

## [0.1.0-alpha.10](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.9...iohr/v0.1.0-alpha.10) (2026-10-05)


### Features

* **cli:** iohr sdk add installs the SDK with the project's package manager ([#144](https://github.com/inorbithr/sdk/issues/144)) ([9473dfe](https://github.com/inorbithr/sdk/commit/9473dfeea44bf1bc9a8e0ad2b91f3d23a8d59699))
* **cli:** iohr sdk config resolves through the Rust SDK's loader, and the Rust target emits the idempotency-key mark and the path template ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **cli:** the C# target emits the path template and the idempotency-key mark ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **conformance:** replay vectors prints the vectors as JSON for drivers without a YAML reader ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **csharp:** calls have a total timeout of 120 s by default (TotalTimeout), every attempt and wait included ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** Client.Load with one precedence (code, environment, the iohr config file, defaults), Describe() and ConfigException problems ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** proxy, CA bundle, mTLS, pinning and connect timeout on SocketsHttpHandler, or a caller's HttpClient or handler ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** the credential chain (env, token and key secret files, the iohr login) with CachedToken, TokenFile, CliToken, ChainedCredential and DefaultCredential ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** the named middleware pipeline with its built-ins, Middleware.FromHandler, ILogger logging, ActivitySource and Meter, rate limits and the retry budget ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **go:** calls have a total timeout of 120 s by default (WithTotalTimeout), every attempt and wait included ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** inorbit.Load with one precedence (code, environment, the iohr config file, defaults), Describe and ConfigError problems ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** OpenTelemetry spans and metrics in the optional module github.com/inorbithr/sdk/go/otel ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** proxy, CA bundle, mTLS, pinning and connect timeout through the SDK's own http.Transport, or a caller's client or RoundTripper ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** the credential chain (env, token and key secret files, the iohr login) with CachedToken, TokenFile, CliToken, ChainedCredential and DefaultCredential ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** the named middleware pipeline over http.RoundTripper with its built-ins, log/slog logging, rate limits and the retry budget ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **java:** calls have a total timeout of 120 s by default, and the attempt timeout covers the body ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** Client.load with one precedence (code, environment, the iohr config file, defaults), describe() and ConfigException problems ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** proxy, CA bundle, mTLS, pinning and connect timeout through the SDK's own ProxySelector and SSLContext ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** the credential chain (env, token and key secret files, the iohr login) with TokenFile, CliToken, CachedToken, ChainedCredential and DefaultCredential ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** the named middleware pipeline with its built-ins, System.Logger logging, optional OpenTelemetry, rate limits and the retry budget ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **py:** calls have a 120 s total deadline by default (total_timeout) ([3c8075d](https://github.com/inorbithr/sdk/commit/3c8075d922cd8341dd80815636d7936d56815cdd))
* **py:** load, the credential chain, the middleware pipeline, transport settings, logging and OpenTelemetry (M6) ([3c8075d](https://github.com/inorbithr/sdk/commit/3c8075d922cd8341dd80815636d7936d56815cdd))
* **py:** writes that take Idempotency-Key are retried, with one key per call ([3c8075d](https://github.com/inorbithr/sdk/commit/3c8075d922cd8341dd80815636d7936d56815cdd))
* **rust:** calls have a total timeout of 120 s by default (total_timeout), and a Retry-After longer than retry_after_max ends the call ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** Client::load with one precedence (code, environment, the iohr config file, defaults), describe() and ConfigError problems ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** proxy, CA bundle, mTLS, pinning and connect timeout through the SDK's own rules, or a caller's reqwest::Client ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** the credential chain (env, token and key secret files, the iohr login) with TokenFile, CliToken, CachedToken, ChainedCredential and DefaultCredential ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** the named middleware pipeline with its built-ins, tracing logging, OpenTelemetry behind the otel feature, rate limits and the retry budget ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **ts:** calls have a total timeout of 120 s by default (totalTimeout), every attempt and wait included ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** Client.load with one precedence (code, environment, the iohr config file, defaults), describe() and ConfigError problems ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** proxy, CA bundle, mTLS, pinning and connect timeout through a node:https transport, or a caller's fetch or dispatcher ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** the credential chain (env, token and key secret files, the iohr login) with CachedToken, TokenFile, CliToken, ChainedCredential and DefaultCredential ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** the named middleware pipeline with its built-ins, logging, OpenTelemetry, rate limits and the retry budget ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))


### Bug fixes

* **conformance:** the replay server hands back action options and rewrite ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **py:** the attempt timeout covers the body; the socket sends an iohr- request id and uses the proxy; the token exchange sends the SDK's user agent; from_env normalises profile names ([3c8075d](https://github.com/inorbithr/sdk/commit/3c8075d922cd8341dd80815636d7936d56815cdd))
* **rust:** the socket task ends when the client is dropped ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))


### Dependencies

* **rust:** tokio-tungstenite 0.30, pkcs8 0.11 and fresh lockfiles ([#135](https://github.com/inorbithr/sdk/issues/135)) ([d13b761](https://github.com/inorbithr/sdk/commit/d13b761ad7b96924c1d14bea08ecd3cd3517aace))

## [0.1.0-alpha.9](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.8...iohr/v0.1.0-alpha.9) (2026-10-05)


### Features

* **cli:** iohr sdk config prints the SDKs' effective configuration ([#110](https://github.com/inorbithr/sdk/issues/110)) ([e4a9d9c](https://github.com/inorbithr/sdk/commit/e4a9d9cef7652fc179f384074b3e42cb801d23a5))
* **cli:** the generator marks operations that take Idempotency-Key ([#111](https://github.com/inorbithr/sdk/issues/111)) ([1390526](https://github.com/inorbithr/sdk/commit/1390526e579e4c213cebca9cff75ab1ed0a3a0fc))


### Bug fixes

* **cli:** sdk config's path rules and vector tests hold on Windows ([#120](https://github.com/inorbithr/sdk/issues/120)) ([c4fdefd](https://github.com/inorbithr/sdk/commit/c4fdefd2912ce8ac709805f55f7e7de82589d9e0))

## [0.1.0-alpha.8](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.7...iohr/v0.1.0-alpha.8) (2026-10-04)


### Features

* **cli:** iohr sdk examples, one compile-checked snippet per operation and language ([#107](https://github.com/inorbithr/sdk/issues/107)) ([f0b871e](https://github.com/inorbithr/sdk/commit/f0b871e14112179b4caed8285e387ad015972af6))
* **cli:** share config.toml with the SDKs, add iohr auth token ([#109](https://github.com/inorbithr/sdk/issues/109)) ([16cb8ef](https://github.com/inorbithr/sdk/commit/16cb8ef26303385fedda8ce3fcbfa4235b2b2606))


### Bug fixes

* **cli:** IOHR_TOKEN wins over the default profile, as the README and ADR 0009 say ([16cb8ef](https://github.com/inorbithr/sdk/commit/16cb8ef26303385fedda8ce3fcbfa4235b2b2606))

## [0.1.0-alpha.7](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.6...iohr/v0.1.0-alpha.7) (2026-10-04)


### Features

* **cli:** iohr connectors and iohr connections (RFC 0044) ([#105](https://github.com/inorbithr/sdk/issues/105)) ([a964223](https://github.com/inorbithr/sdk/commit/a96422376ddcd970c4b18900d1b0349639d497ba))
* streams in all six languages, over server-sent events or the /v1/ws socket (M5) ([#96](https://github.com/inorbithr/sdk/issues/96)) ([7ff73fe](https://github.com/inorbithr/sdk/commit/7ff73fe986f4706299d188ef9a4d5273dc0ca7ee))

## [0.1.0-alpha.6](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.5...iohr/v0.1.0-alpha.6) (2026-10-04)


### Features

* **cli:** iohr api --all walks every page of a list into one answer ([#95](https://github.com/inorbithr/sdk/issues/95)) ([ea86b94](https://github.com/inorbithr/sdk/commit/ea86b94fc45b30f2833c1ad850efb28e20759a20))

## [0.1.0-alpha.5](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.4...iohr/v0.1.0-alpha.5) (2026-10-04)


### ⚠ BREAKING CHANGES

* request fields are optional in every language (a Rust request is built with ..Default::default(), a Go scalar is a pointer set with inorbit.Ptr); answer fields with presence (messages, optional, oneof) are optional where they used to be typed as always present.

### Features

* the contract states what the sync patched in; requests leave out unset fields ([#85](https://github.com/inorbithr/sdk/issues/85)) ([9eff311](https://github.com/inorbithr/sdk/commit/9eff311ab88bee9873a777b90197c16b471809e2))

## [0.1.0-alpha.4](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.3...iohr/v0.1.0-alpha.4) (2026-10-03)


### Features

* **cli:** iterators over paged lists in TypeScript, Python, Rust, Java and C# ([#66](https://github.com/inorbithr/sdk/issues/66)) ([3a42fd6](https://github.com/inorbithr/sdk/commit/3a42fd63fbfc855a916e8694f0ff5c0fe4c7bab5))
* **csharp:** the C# runtime and target ([#58](https://github.com/inorbithr/sdk/issues/58)) ([bc1e7bb](https://github.com/inorbithr/sdk/commit/bc1e7bbcb0927247e31d754a92020a153489995b))
* **go:** iterators over paged lists, slog logging, Example functions, virtual-clock tests ([#60](https://github.com/inorbithr/sdk/issues/60)) ([069a9cb](https://github.com/inorbithr/sdk/commit/069a9cb0bc516be82d2f290457c1455b752f669b))
* **go:** the Go runtime and target ([#56](https://github.com/inorbithr/sdk/issues/56)) ([8bbebcb](https://github.com/inorbithr/sdk/commit/8bbebcb4870fa86fbd45e55b0fee6bd1ddf871e4))
* **java:** the Java runtime and the Java target ([#59](https://github.com/inorbithr/sdk/issues/59)) ([325f2f9](https://github.com/inorbithr/sdk/commit/325f2f953bfa0e985bf45ff0203ae7dd7452ca78))
* **py:** the Python runtime and target ([#57](https://github.com/inorbithr/sdk/issues/57)) ([e823d2e](https://github.com/inorbithr/sdk/commit/e823d2e9f4e91c29ae14d7ff0efa71be416f9a15))
* **spec:** the contract synced with the platform; unprocessable in every runtime ([#65](https://github.com/inorbithr/sdk/issues/65)) ([69aad11](https://github.com/inorbithr/sdk/commit/69aad110b1e77c4c49241846581959537dfcf15f))
* **ts:** the TypeScript runtime and target ([#46](https://github.com/inorbithr/sdk/issues/46)) ([3a2b2b0](https://github.com/inorbithr/sdk/commit/3a2b2b03f3325bac7a048cd642d6ac5c93f495b3))


### Bug fixes

* **cli:** a new platform error code no longer stops sdk generate ([#61](https://github.com/inorbithr/sdk/issues/61)) ([b6689ef](https://github.com/inorbithr/sdk/commit/b6689ef6611663164e03d113b7684b9ead0ec7e2))

## [0.1.0-alpha.3](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.2...iohr/v0.1.0-alpha.3) (2026-10-03)


### Features

* **cli:** a language-neutral generator core; six languages known ([#41](https://github.com/inorbithr/sdk/issues/41)) ([e6952a0](https://github.com/inorbithr/sdk/commit/e6952a000a7c7193906c16da2f0440918da317e1))
* **cli:** iohr ext (RFC 0028) and iohr domains (RFC 0030) ([#40](https://github.com/inorbithr/sdk/issues/40)) ([dbae5c8](https://github.com/inorbithr/sdk/commit/dbae5c8cb211c7decb652df910f90d476e35951d))
* **cli:** iohr lab check, the lab document checks on your own repository ([#53](https://github.com/inorbithr/sdk/issues/53)) ([9cb02be](https://github.com/inorbithr/sdk/commit/9cb02be784c5f45b9096228defd192cf00755e12))
* **cli:** sdk generate from profiles, sdk check with iohr.lock, profile account ([#36](https://github.com/inorbithr/sdk/issues/36)) ([748f30a](https://github.com/inorbithr/sdk/commit/748f30ab65499dd8b9d31a3e909a75bb6dfbd1ae))
* **cli:** the generator with the Rust target; the crate's public surface is generated ([#35](https://github.com/inorbithr/sdk/issues/35)) ([d4e77cf](https://github.com/inorbithr/sdk/commit/d4e77cffd5d7a20a77a484fdd56a59fbcac1dd4d))


### Bug fixes

* **cli:** the ext tests build on Windows ([#51](https://github.com/inorbithr/sdk/issues/51)) ([1ee3f8b](https://github.com/inorbithr/sdk/commit/1ee3f8b004c863def0540132b6e81d4d49cef056))
* **cli:** the Homebrew formula's description passes brew audit ([#29](https://github.com/inorbithr/sdk/issues/29)) ([7535a35](https://github.com/inorbithr/sdk/commit/7535a35602182773be9b08e8353d4c55ba6248f9))
* **cli:** the last Unix-only import in the ext tests moves with its helper ([#52](https://github.com/inorbithr/sdk/issues/52)) ([87c3cfd](https://github.com/inorbithr/sdk/commit/87c3cfdb3838eb115b5da6d714414e47c456c75d))

## [0.1.0-alpha.2](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.1...iohr/v0.1.0-alpha.2) (2026-10-02)


### Bug fixes

* **cli:** the release scripts live in cli/release ([#27](https://github.com/inorbithr/sdk/issues/27)) ([eef71a8](https://github.com/inorbithr/sdk/commit/eef71a87b8f8bf395e50279339c9f9190507b244))

## 0.1.0-alpha.1 (2026-10-02)


### Features

* **cli:** install.sh and install.ps1, checked against the release ([#18](https://github.com/inorbithr/sdk/issues/18)) ([7cff577](https://github.com/inorbithr/sdk/commit/7cff57755e379325b9ca04232ea6767e8a52b6bd))
* **cli:** iohr login in a browser or with a device code ([#15](https://github.com/inorbithr/sdk/issues/15)) ([5387abc](https://github.com/inorbithr/sdk/commit/5387abc8d5b838c12b92729485fc375e35152918))
* **cli:** iohr signs in with an API token, keeps profiles and calls the API ([#14](https://github.com/inorbithr/sdk/issues/14)) ([6c3189c](https://github.com/inorbithr/sdk/commit/6c3189c62df5ebfeb612be21a244f5802509872c))
