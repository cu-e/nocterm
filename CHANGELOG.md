# Changelog

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
