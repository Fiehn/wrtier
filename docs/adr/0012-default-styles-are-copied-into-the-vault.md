# Default Styles are copied into the Vault

The Styles, Looks and Story elements the app ships with are written into the Vault's `styles/` folder as ordinary files when the Vault is made, and from then on belong to the Vault. Works record the content of every Style they use ([ADR 0004](0004-works-record-styles-but-only-reference-fonts.md)), so if the defaults lived inside the app, two Devices running different app versions would each see a different "Paperback" and record it back and forth on every edit, with a Style update notice each time. In the Vault, every Device reads the same files whatever its version, and the Author can edit them like any other Style.

A newer app version never overwrites them. A default the Vault has never had (a new Look, say) is written once; `vault.db` records which defaults were written, so one the Author deleted doesn't come back. An improved version of an existing default is only offered ("A newer Paperback is available. Compare / Use it").

## Considered Options

- **Read-only defaults inside the app, copied to the Vault only to edit them.** No files to manage, but Works ping-pong between versions whenever Devices run different app versions.
