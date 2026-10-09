# Changelog

## [3.1.0](https://github.com/cu-e/nocterm/compare/v3.0.0...v3.1.0) (2026-10-09)


### Features

* **agent:** connect shown chats and recover failed sessions ([4046980](https://github.com/cu-e/nocterm/commit/4046980d6a7d750db40e00b28d56d98c633d0f3e))


### Bug Fixes

* **agent-runtime:** load history on demand and order quit persistence ([940bd7b](https://github.com/cu-e/nocterm/commit/940bd7b6831c429be1e265394c0d8104ce3fad06))
* **agent:** decode historical tool wrappers tolerantly ([b43791d](https://github.com/cu-e/nocterm/commit/b43791d444bed287325e10f06e096b7d1231dc95))
* **agent:** keep terminal tools working for Hermes and after rebuilds ([bbe95e1](https://github.com/cu-e/nocterm/commit/bbe95e1ff98e10b5ecc23c6d6d9b833a4dde8724))
* **agent:** keep terminal tools working for Hermes and after rebuilds ([5784459](https://github.com/cu-e/nocterm/commit/5784459ea008cd3799f2124fcd8c0d3424cb12ab))
* **agent:** persist bridge-owned tool outcomes ([9efd3af](https://github.com/cu-e/nocterm/commit/9efd3af4dd909adfe747aa48fea4bf2ab768fedd))
* **agent:** render tool row sections independently ([a73db59](https://github.com/cu-e/nocterm/commit/a73db5973462bd349ba37cdc8de1a1c9c26ae35a))
* **agent:** replace truncated previews with complete tool results ([9ddba71](https://github.com/cu-e/nocterm/commit/9ddba71133ed06e615c80277138adcfc9cddb522))
* **agent:** show rejected tool requests and Codex errors ([93e8566](https://github.com/cu-e/nocterm/commit/93e85664880c1dc5942a97aa479fc7a284e1f82c))
* **containers-ui:** bind destructive confirmation to its host ([fbdf7ae](https://github.com/cu-e/nocterm/commit/fbdf7aea0c01513a35d878b678700784685dffbe))
* **local:** cancel local commands and start them off the GUI thread ([8821134](https://github.com/cu-e/nocterm/commit/88211340d83793751d1e457a139dad87f7732be2))
* **monitor:** bound frame lines and deliver frames one at a time ([90975ad](https://github.com/cu-e/nocterm/commit/90975ad129e696a471132880d16ceb997c2048c3))
* **ui:** order layout and settings writes on quit ([f79adcd](https://github.com/cu-e/nocterm/commit/f79adcdd94bf06150f608f8b8c51b5af3fd107de))


### Documentation

* record architecture decisions and the layer rules ([4991b30](https://github.com/cu-e/nocterm/commit/4991b30f33aa7b9c939fd717b1ec21352eb0feb1))
* split the architecture notes into one document per layer ([e78d108](https://github.com/cu-e/nocterm/commit/e78d1084a61d0f2fd992ad5b1f216bbe773f6752))

## [3.0.0](https://github.com/cu-e/nocterm/compare/v2.0.0...v3.0.0) (2026-10-08)


### ⚠ BREAKING CHANGES

* **agent:** Linux AI agents require systemd 254 or newer and a running systemd user manager. Managed agent startup fails closed when this prerequisite is unavailable.

### Features

* **agent:** configure provider permission prompts ([0b446f1](https://github.com/cu-e/nocterm/commit/0b446f1c83ab85a24a8fb41c47818505923d7ec6))
* **agent:** persist composer drafts and add message navigation ([3195347](https://github.com/cu-e/nocterm/commit/31953473e7686adaa720934f2ebe58bdd8ebec38))


### Bug Fixes

* **acp:** classify missing rollout as unavailable restore error ([c2c102c](https://github.com/cu-e/nocterm/commit/c2c102c6f63b440bb1c1905e75058271e76ef839))
* **agent:** bound live resources independently of chat history ([bf4abcf](https://github.com/cu-e/nocterm/commit/bf4abcfc98993759c9d98abea4b4b04c9638d000))
* **agent:** dismiss usage card on escape and focus loss ([af8dd5b](https://github.com/cu-e/nocterm/commit/af8dd5b3ddc08f2291d088f656b39eba8a44ce9d))
* **agent:** render provider terminal calls and results ([12face9](https://github.com/cu-e/nocterm/commit/12face983be72e5c8d82608cf498d1d2822b842d))
* **terminal:** preserve interrupt key routing ([9d722cc](https://github.com/cu-e/nocterm/commit/9d722cca582eed2e35b8e28c1aba960a4d599b46))
* **terminal:** scroll selection beyond viewport edges ([04c4320](https://github.com/cu-e/nocterm/commit/04c4320262e9322cf6b12ef9b66c41c21f7866f2))
* **vt:** track pointer interaction state ([163a215](https://github.com/cu-e/nocterm/commit/163a2158870717b13fac3c98639edbdb0a21d59e))

## [2.0.0](https://github.com/cu-e/nocterm/compare/v1.5.0...v2.0.0) (2026-10-07)


### ⚠ BREAKING CHANGES

* **terminal:** run_command requires a known, empty shell prompt. Use exec_command on SSH sessions without shell integration.

### Features

* **agent:** execute scoped host commands ([e4cb88c](https://github.com/cu-e/nocterm/commit/e4cb88c342572438c48dd9bea3ba5ef145eb2d22))
* **files:** add editable paths with asynchronous completion ([3ec6e07](https://github.com/cu-e/nocterm/commit/3ec6e070ece90d4396a4b55f57e2f59915291bf6))
* **files:** merge explorer path interactions into dev ([9e45d43](https://github.com/cu-e/nocterm/commit/9e45d4388f21897a6f1b0b5c0e81dc5a7c9db6d3))
* **terminal:** improve terminal interaction ([9ac83ab](https://github.com/cu-e/nocterm/commit/9ac83ab96f18e588cc1d958be0a091fed4a3970c))
* **terminal:** paste dragged explorer paths into terminals ([e0239cb](https://github.com/cu-e/nocterm/commit/e0239cb5a74a769fcb234a7b99de6c7acc0adf73))
* **workspace:** add local terminal tabs with header controls ([f52a805](https://github.com/cu-e/nocterm/commit/f52a8057ed600e47162693ef0e5a1a74b35c1369))


### Bug Fixes

* **agent:** clarify terminal tool calls in chat ([c3a8b2c](https://github.com/cu-e/nocterm/commit/c3a8b2cd4a33b0f984a6f0af1e6f3c56673de649))
* **ai:** preserve credential masks across workspace aliases ([2e2ff10](https://github.com/cu-e/nocterm/commit/2e2ff10ee8c9d08f3ec661e3276824aa8b4b61aa))
* **device-unlock:** report fingerprint attempt state ([9d30482](https://github.com/cu-e/nocterm/commit/9d30482573fa1603fcdcc5eb82ccadff1743008f))
* **files:** refine explorer directory navigation ([f85bd77](https://github.com/cu-e/nocterm/commit/f85bd7708179f193f56efac5407392c87123be84))
* **ssh:** prevent blocked program startup ([9deef2f](https://github.com/cu-e/nocterm/commit/9deef2fa3ede529f2b3239932fc672c4d1843ec1))
* **terminal:** guard agent commands with prompt ownership ([2d270d5](https://github.com/cu-e/nocterm/commit/2d270d5dce08e08177b40ecb695a13c181b69bdd))
* **vault-broker:** persist fingerprint attempt limits ([586f659](https://github.com/cu-e/nocterm/commit/586f6595e0d113d9a28e35ef8b88c4babe49362a))
* **workspace:** keep zoomed terminal groups interactive ([ef78bbd](https://github.com/cu-e/nocterm/commit/ef78bbd73fc56980beae63b31a9f436ebe71894d))
* **workspace:** stop dock zoom event feedback ([77b3c05](https://github.com/cu-e/nocterm/commit/77b3c055c14ca32cdc8c03b3d39c104430e66f9b))
* **workspace:** unify terminal tab operations ([b9ef593](https://github.com/cu-e/nocterm/commit/b9ef593ea95f85f734528ccdd926b36a7118f049))


### Performance

* **terminal:** reuse unchanged terminal frames ([40a7a8a](https://github.com/cu-e/nocterm/commit/40a7a8a15c00b41a567aa081a56ca894b0d53cad))
* **ui:** add opt-in Linux retained rendering ([a8ab0aa](https://github.com/cu-e/nocterm/commit/a8ab0aa1d6094fbdc016270cdbe2e57c6051b8bc))
* **ui:** retain Windows renderer frames ([0b25b4b](https://github.com/cu-e/nocterm/commit/0b25b4b730e717cdcc02d3bc96b5c35890391549))


### Documentation

* **ui:** mark shared GPUI fork modifications ([a239938](https://github.com/cu-e/nocterm/commit/a2399389df228f604ea6cfc334ba7878c23f7d56))
* **ui:** mark the spring animation fork change ([2891ad4](https://github.com/cu-e/nocterm/commit/2891ad4eade0aba5e7017bf016737e276c0dc432))

## [1.5.0](https://github.com/cu-e/nocterm/compare/v1.4.2...v1.5.0) (2026-10-07)


### Features

* **agent:** render tool input and collapse titles ([c92bb61](https://github.com/cu-e/nocterm/commit/c92bb61b21e0477171e39201677fe6b6c2a82613))
* **agent:** render tool input and collapse titles ([5eaa820](https://github.com/cu-e/nocterm/commit/5eaa82085026e3f9c60f591be70ff881be123384))
* **snippets-ui:** organise attachments into a searchable tree ([b9b5c13](https://github.com/cu-e/nocterm/commit/b9b5c1381789462e60741554222f44d0a58bd068))
* **snippets-ui:** run snippets in the focused terminal ([df4a305](https://github.com/cu-e/nocterm/commit/df4a305bbbd21e8a56c3eb481eaf9a0db73d763a))
* **snippets:** add a contextual snippet library ([e678e6f](https://github.com/cu-e/nocterm/commit/e678e6fff8761718ec502e14c13beb7fcd7f24bf))
* **snippets:** add contextual snippet library ([440bf30](https://github.com/cu-e/nocterm/commit/440bf30293c3e0e3f798fb2b18b3a43e5c9fd125))
* **workspace:** show recent servers in the empty state ([721f3b9](https://github.com/cu-e/nocterm/commit/721f3b95a318e544a88e552427de0b49af0d217f))


### Bug Fixes

* **keymap:** expose snippet commands in the palette ([2a9145d](https://github.com/cu-e/nocterm/commit/2a9145d2e3ba267ff1fff2fc76cae8bb434504b4))
* **snippets-ui:** preserve modal ownership when saving ([b7ab9c8](https://github.com/cu-e/nocterm/commit/b7ab9c85b03ae07c9c3b13586d9aa1c3989050e4))
* **snippets-ui:** remove attachment labels from snippet rows ([9986c4c](https://github.com/cu-e/nocterm/commit/9986c4ca7e2e0d3416bc1466f5689088ac126a24))
* **ui:** preserve source appearance in drag previews ([c063ff3](https://github.com/cu-e/nocterm/commit/c063ff374688f74431ed945046a12ff4fe56508f))
* **vault-broker:** report failures and retry fingerprint verification ([ca2c3cc](https://github.com/cu-e/nocterm/commit/ca2c3cc95c68e3a102b52af67fce11c4c29c4815))
* **vault-broker:** report failures and retry fingerprint verification ([963f044](https://github.com/cu-e/nocterm/commit/963f044c6a5f46ecc33564937a94e6562660d213))
* **workspace:** place recent server icons before names ([0f9d7de](https://github.com/cu-e/nocterm/commit/0f9d7de30ed84c683f7767d5f2651853d56313a2))

## [1.4.2](https://github.com/cu-e/nocterm/compare/v1.4.1...v1.4.2) (2026-10-06)


### Bug Fixes

* keep license notices valid across release version bumps ([b655b61](https://github.com/cu-e/nocterm/commit/b655b61f6b22ca4d3e4a65e4ddad48616e390041))
* keep license notices valid across release version bumps ([4fd4c5b](https://github.com/cu-e/nocterm/commit/4fd4c5b5671ba77ec35495cfe4b484f3091844db))

## [1.4.1](https://github.com/cu-e/nocterm/compare/v1.4.0...v1.4.1) (2026-10-05)


### Bug Fixes

* **acp:** retry interrupted bridge reads and disconnect before cancel ([2a431f3](https://github.com/cu-e/nocterm/commit/2a431f3b92adae9f90729f1d98d3ea321107d879))

## [1.4.0](https://github.com/cu-e/nocterm/compare/v1.3.0...v1.4.0) (2026-10-05)


### Features

* **workspace:** add floating card layout with pill tabs ([ccf1f92](https://github.com/cu-e/nocterm/commit/ccf1f920e456ff15527817ae4b1223b44d92f5af))
* **workspace:** add floating card layout with pill tabs ([e165ca7](https://github.com/cu-e/nocterm/commit/e165ca714d71df34e2c58eec9851725b8603dfd7))


### Bug Fixes

* **workspace:** align floating tab bar with the agent header ([20013a2](https://github.com/cu-e/nocterm/commit/20013a27c3e9b603b8a694c1387e3b843780763d))
* **workspace:** keep card corner mask out of the scroll region ([1dee04f](https://github.com/cu-e/nocterm/commit/1dee04fcc4e086613285c291971fbe8a24ebbb37))

## [1.3.0](https://github.com/cu-e/nocterm/compare/v1.2.0...v1.3.0) (2026-10-05)


### Features

* **agent:** improve ACP commands, queued prompts and chat continuity ([02c34a2](https://github.com/cu-e/nocterm/commit/02c34a2166bfce75ed3b4e90d4087bf948639fc3))
* **agent:** improve ACP commands, queued prompts and chat continuity ([1fb4b00](https://github.com/cu-e/nocterm/commit/1fb4b004751c22c11e9361273c7236c264cacfb3))


### Performance

* batch terminal output events and cache idle workspace panels ([6d5c4cd](https://github.com/cu-e/nocterm/commit/6d5c4cd79fceee00a69129d8c09f796c46dda1f3))
* batch terminal output events and cache idle workspace panels ([a090ec0](https://github.com/cu-e/nocterm/commit/a090ec0aeab50e38a8d8812730d4f5fc7dd7ba7f))

## [1.2.0](https://github.com/cu-e/nocterm/compare/v1.1.0...v1.2.0) (2026-10-05)


### Features

* **connections:** reorder saved servers by drag and drop ([73a319b](https://github.com/cu-e/nocterm/commit/73a319b89748b9ac4c3eee2144eabecdd6490b10))
* **containers-ui:** add containers panel ([64c155e](https://github.com/cu-e/nocterm/commit/64c155eaa0743da471d7bd5e957757e57fc5f6b0))
* **files:** add an Explorer page to Settings ([7c017d2](https://github.com/cu-e/nocterm/commit/7c017d2aaf25c4227a9b8c50305dae23df2b857f))
* **files:** count remote folders and skip home and excluded folders ([4adefd9](https://github.com/cu-e/nocterm/commit/4adefd9cbcd41af3eb55a96e9baa32e6ae7296c6))
* **files:** open files with the chosen programs and in the file manager ([fc37d16](https://github.com/cu-e/nocterm/commit/fc37d16e70253bc0a6f29d38aa7aec30709a60f4))
* **files:** show transfer progress in Explorer and refresh when done ([afdec19](https://github.com/cu-e/nocterm/commit/afdec192259efec59cd0822547d2f9f214cb0486))
* **settings:** add explorer indexing and file opener settings ([a87481e](https://github.com/cu-e/nocterm/commit/a87481eb159a303d58ed4dafbc62ca6169eefeab))
* **workspace:** open a program on a connected host in a new tab ([5b72304](https://github.com/cu-e/nocterm/commit/5b72304c49213e0eb9305217a96b2407934bf6f9))


### Bug Fixes

* **connections:** keep a server in place when dropped on its own row ([3fc1835](https://github.com/cu-e/nocterm/commit/3fc183527a4defc36d76809b4e07ddc867d3c858))
* **files:** keep folder statistics and transfer refreshes consistent ([a9e13ff](https://github.com/cu-e/nocterm/commit/a9e13ffc2db9ba6ee1e888b85cfcc6b34588933e))
* **files:** name the folder that could not be listed ([b8160eb](https://github.com/cu-e/nocterm/commit/b8160eb453cdcf178d429656be47d080dee4245b))
* **local:** report the zsh working directory without a stray %25 ([0b51d2a](https://github.com/cu-e/nocterm/commit/0b51d2a7942b3e8dc24a6d753ca13f28420f8d09))
* **settings:** stop excluding common folder names from folder sizes ([929120a](https://github.com/cu-e/nocterm/commit/929120ac65787ccdbebd880b1ade014e229d1c99))
* **ssh:** make connection recovery actionable ([c3e585c](https://github.com/cu-e/nocterm/commit/c3e585c50e7f4c79dcf4c81ed7bd08602e79c994))
* **vault-ui:** submit the unlock password on Enter instead of closing ([b1bb434](https://github.com/cu-e/nocterm/commit/b1bb4346c2e630d7f4b8527095b6a53bc964cbec))
* **workspace:** open host programs from the active tab's sign-in ([243748a](https://github.com/cu-e/nocterm/commit/243748a23aee3c55d8b12caf8519cee4ab187e40))


### Documentation

* align release guidance with master ([2672496](https://github.com/cu-e/nocterm/commit/26724962dfe2f735db8804d12f9a0347c5b07d95))

## [1.1.0](https://github.com/cu-e/nocterm/compare/v1.0.0...v1.1.0) (2026-10-05)


### Features

* **monitor:** implement host resource monitoring and UI ([16defde](https://github.com/cu-e/nocterm/commit/16defde74d6eda78ec5c196881a4ef4f53b9ca1c))
* **monitor:** implement host resource monitoring and UI ([16defde](https://github.com/cu-e/nocterm/commit/16defde74d6eda78ec5c196881a4ef4f53b9ca1c))
* **monitor:** implement host resource monitoring and UI ([cc76b49](https://github.com/cu-e/nocterm/commit/cc76b498987da1b2a8296d2e12154cc5510079c2))


### Bug Fixes

* **app:** link the MSVC runtime statically and stop panicking on quit ([f17648b](https://github.com/cu-e/nocterm/commit/f17648bd4a601d0318b4ad1101e10d61fed93ac0))

## 1.0.0 (2026-10-04)


### ⚠ BREAKING CHANGES

* **app:** internal settings/profile mutation APIs and credential-association callbacks now return Tasks. Callers must await durable completion; settings drafts must retain their published revision.

### Features

* **acp:** add model selection and client tests ([bde561c](https://github.com/cu-e/nocterm/commit/bde561c5309e6bed6d8cc2e075ae35a6df8cf264))
* **agent:** add ACP chat panel and terminal context ([971088e](https://github.com/cu-e/nocterm/commit/971088ee523abef4422f51c003852554534973c2))
* **agent:** answer background sign-in prompts in the chat ([9a89124](https://github.com/cu-e/nocterm/commit/9a891242b84272f74c23830736d9880435340d04))
* **agent:** redesign the agent menu, history and approvals ([7d549ef](https://github.com/cu-e/nocterm/commit/7d549ef4164cac1d8331efd9e061725eb7f8bfee))
* **agent:** unlock the vault from the chat for background sign-in ([b20f5ab](https://github.com/cu-e/nocterm/commit/b20f5abec1e23e3446c8188983085809679364fa))
* **ai:** add usage, history and sandbox models ([d1dcfa2](https://github.com/cu-e/nocterm/commit/d1dcfa2b5e17e01cf559b506e80ee8d46ca86014))
* **app:** add titlebar menus and window actions ([4f1e18f](https://github.com/cu-e/nocterm/commit/4f1e18f7af230161ac1318c31a077d3c479cbab5))
* **app:** integrate operational notifications ([9b8d091](https://github.com/cu-e/nocterm/commit/9b8d091be2d6c3e15a67200cafdba95fec221a20))
* **app:** integrate packaging and vault chat unlock ([9b9de41](https://github.com/cu-e/nocterm/commit/9b9de41e38b3e4de9a92c44b9217ee1b88be485f))
* **app:** introduce extensible SSH workspace ([b86e26a](https://github.com/cu-e/nocterm/commit/b86e26afec378109d89e4a190cbb162ab3c883b2))
* **app:** wire keymap, agent auth and shortcut tests ([2d7250e](https://github.com/cu-e/nocterm/commit/2d7250ec18e849a003b7a5644e6ebb32290ab9c8))
* **connections:** move saved profiles between folders ([8034244](https://github.com/cu-e/nocterm/commit/80342445b52cb3da5c89ff213f76d240b5a11150))
* **connections:** organize connection editor sections ([7f9a884](https://github.com/cu-e/nocterm/commit/7f9a8843b01277d65a353b4a662910da2615e80d))
* **connections:** rename and delete groups from the sidebar ([e360f42](https://github.com/cu-e/nocterm/commit/e360f42dec23717a2790628c8274d994a71d9adf))
* **connections:** split the profile editor and add host facts ([61ea3b9](https://github.com/cu-e/nocterm/commit/61ea3b93532cd6101c4db440b699cbbe07be6c9f))
* **core:** add private runtime directory ([2a8e61f](https://github.com/cu-e/nocterm/commit/2a8e61f58b1242e70018d73ac5a778961598e950))
* **core:** extend paths and persistence helpers ([006115e](https://github.com/cu-e/nocterm/commit/006115e17a86f8cc8217c6992139ced74e415028))
* **design:** add tokens for the redesigned panels ([c3b6b57](https://github.com/cu-e/nocterm/commit/c3b6b5771e92b60fc83407205b86b23c2b18007f))
* **device-unlock:** activate the broker on demand and explain setup ([4fe436d](https://github.com/cu-e/nocterm/commit/4fe436dbdf11cc4d9bb1a671e8a223f5a411f76d))
* **files:** add contextual file management ([7ff699c](https://github.com/cu-e/nocterm/commit/7ff699c8bda375720c51983b20d944e8c6ea668c))
* **keymap:** extract key bindings into keymap and keymap-ui crates ([76ef017](https://github.com/cu-e/nocterm/commit/76ef0172250493f86c5e6438c047a6d22746f3a0))
* **session:** add remote file management contracts ([590ed6f](https://github.com/cu-e/nocterm/commit/590ed6fb6572e41195d59d0e67a2158cee9af363))
* **settings-ui:** host the vault in tabbed settings ([1ec53c9](https://github.com/cu-e/nocterm/commit/1ec53c9334a896d82630ef0dc6f01dbc548d6f0b))
* **settings-ui:** rebuild settings pages with shared fields ([d18f44b](https://github.com/cu-e/nocterm/commit/d18f44bcc1389409b31889de22679be2410287c7))
* **settings:** add ai settings schema ([16133ab](https://github.com/cu-e/nocterm/commit/16133abf95986e159a70b9e3236a5898fa0aa527))
* **settings:** extend the ai settings schema ([bb9515b](https://github.com/cu-e/nocterm/commit/bb9515ba83da7efeff98833f53280a7aabb3b48c))
* **terminal:** add search mode controls ([2ec9409](https://github.com/cu-e/nocterm/commit/2ec940995383cb2f8b59c805690b33708d8bcd4d))
* **terminal:** add searchable session command controls ([9b4bb24](https://github.com/cu-e/nocterm/commit/9b4bb249c6fca50f253330ffd5461813d93a0f73))
* **terminal:** extract secret prompts from the terminal view ([c32cb57](https://github.com/cu-e/nocterm/commit/c32cb5714fb076b06e2b9afbf151dd40e42caa14))
* **themes:** add selectable zed-compatible color themes ([67a1005](https://github.com/cu-e/nocterm/commit/67a10054c611372bd0aeec6776bb7d2671b06f10))
* **ui:** add form, layout and notice bar primitives ([f5de5e5](https://github.com/cu-e/nocterm/commit/f5de5e52e40d4b716463755b2550e196ed23c866))
* **ui:** add shared interaction components ([457c446](https://github.com/cu-e/nocterm/commit/457c44617d0770eb76fb9066f1a4e295fc8ea23b))
* **vault-broker:** keep fingerprint keys across Nocterm restarts ([e2df3cf](https://github.com/cu-e/nocterm/commit/e2df3cf4051569c7607ca1a9da1d3e53135e146b))
* **vault-ui:** split the vault page into view, render and unlock ([c0227a2](https://github.com/cu-e/nocterm/commit/c0227a214b253afac513f88354a85f012be2da9a))
* **vault-ui:** unlock with fingerprint from the unlock dialog ([4f1b7bf](https://github.com/cu-e/nocterm/commit/4f1b7bf045ab7f71a6ca35b570be4480f8aef174))
* **vault:** add authenticated device unlock providers ([68d0731](https://github.com/cu-e/nocterm/commit/68d07318d357dbcf18fc3956a45ca5ba6633c50c))
* **vault:** keep device unlock on until the user turns it off ([d07edb8](https://github.com/cu-e/nocterm/commit/d07edb8747c0ca0974807c669c2b7597e92e184b))
* **vault:** re-register session device unlock after password unlock ([41a3c10](https://github.com/cu-e/nocterm/commit/41a3c10c8f34601428dadad9583a141501897474))
* **vt:** add incremental Unicode terminal search ([9d1d158](https://github.com/cu-e/nocterm/commit/9d1d158220d984cd7765a10bb66b32fbf5edc9f7))
* **vt:** extract bounded terminal text ([8408915](https://github.com/cu-e/nocterm/commit/8408915b5588a619663a530a809eb2551eadb41f))
* **workspace:** add command palette and split workspace layout ([46bf06f](https://github.com/cu-e/nocterm/commit/46bf06ffc2a91f686e0340de8be7c3082445311d))
* **workspace:** add contextual tab controls ([a5b24e1](https://github.com/cu-e/nocterm/commit/a5b24e1d87d6c3788d587ce14e040f9aa41c54cb))
* **workspace:** route menus through focused item containers ([b71360f](https://github.com/cu-e/nocterm/commit/b71360f4385e526feb835bdcc9a4798e69655e6a))


### Bug Fixes

* **app:** harden audited service boundaries ([adb0c23](https://github.com/cu-e/nocterm/commit/adb0c23084b9f6c35335e9c2f27eefe8fab74878))
* **app:** name native workspace windows ([e70129e](https://github.com/cu-e/nocterm/commit/e70129e1dd030776d612bd6ed2a52084ca86f865))
* **connections:** preserve independent persistence errors ([1a4d7d3](https://github.com/cu-e/nocterm/commit/1a4d7d3b2cb1ecf159afdda4b2a83aa6ba150069))
* **connections:** replace stale editors when reloading ([1be2e96](https://github.com/cu-e/nocterm/commit/1be2e96ced4862d76f64dd0b9a74ea72a825e1a2))
* **settings-ui:** retain focus after menu dismissal ([655596b](https://github.com/cu-e/nocterm/commit/655596bd4222b78cf2ed4f2dbc1561bb1b577fc1))
* **transfers:** keep batch progress in creation order ([2f2fd0e](https://github.com/cu-e/nocterm/commit/2f2fd0eabb65f6023b6c229d33de1fb4359b4a9e))
* **ui:** composite imported terminal colors over component defaults ([5cfddaa](https://github.com/cu-e/nocterm/commit/5cfddaa63ab531fb8649c0a57d3a9bb038ae37e3))
* **vault-broker:** expire keys despite bus traffic ([d4c1916](https://github.com/cu-e/nocterm/commit/d4c191619059af2b7e2eccda5820aa400b37c811))
* **vault-broker:** reject stale verification completions ([af3bdf1](https://github.com/cu-e/nocterm/commit/af3bdf1866f976432d48b0f8242db8acc5eaa49c))
* **vault:** cancel outstanding native authentication ([9464935](https://github.com/cu-e/nocterm/commit/94649352af140af111806ba570cbe9271f6757c6))
* **vt:** bound retained terminal cell metadata ([0364316](https://github.com/cu-e/nocterm/commit/0364316f6b66dbc9291757ae65be5bf5faef5c9f))
* **workspace:** fully hide the local terminal dock ([5796185](https://github.com/cu-e/nocterm/commit/5796185fc68fd634acbdba1620fa3eac853bee8c))


### Documentation

* **app:** describe explorer and workspace controls ([0d216dd](https://github.com/cu-e/nocterm/commit/0d216dda4a366e98148c4d18be3a345af36d5cff))
* **app:** document menus and terminal search ([78829a0](https://github.com/cu-e/nocterm/commit/78829a0ec5822238e7b6fba0e08c87add592ef72))
* **app:** record architecture audit follow-up evidence ([159d9f9](https://github.com/cu-e/nocterm/commit/159d9f909b9245c30f605cc4bb00dfe1e5b19aa8))
* describe packaging and automatic fingerprint broker setup ([6c8b8f2](https://github.com/cu-e/nocterm/commit/6c8b8f262d7f19ed148dab987b8e3ac15b7fc61f))
* **ui:** describe interaction behavior and ownership ([1e84e15](https://github.com/cu-e/nocterm/commit/1e84e150a540cc96b4edecc7e2c2af6e792c024a))
* **vault-broker:** describe user-bound fingerprint keys ([15af5d3](https://github.com/cu-e/nocterm/commit/15af5d33538d2953ef30505c17a2eaa559262fc9))
