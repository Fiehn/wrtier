# Mods are built in and kept outside the History

A Mod is an optional feature that ships with the app (the Chapter list, spellcheck), which the Author adds to each Work separately. Mods are never code from anywhere else: a third party's code would run with access to every History and, on Android, every file ([ADR 0006](0006-android-uses-all-files-access.md)). It could also break the promises the app rests on, that only the Author changes the text and that the same version gives the same PDF on every Device. So a Mod only adds ways to see and work on a Work. It never changes the text, the History, or how the Work renders or exports.

Which Mods a Work has is kept in `vault.db` under the Work's ID, one field per Mod, not in the Work's Preamble. Anything in the Preamble is part of the History: returning to a Milestone, or Reverting a Session, would quietly add or remove Mods, and every switch would show up in the History view. Keyed by the Work's ID, the list follows renames and the trash. A `.db` copied into another Vault opens without its Mods, which costs nothing, since they don't change the text. With one field per Mod, two Devices switching different Mods both keep their switch, and an older app never removes a Mod it doesn't know.

## Considered Options

- **Installable add-ons in the Vault.** Lets anyone add features, but needs a sandbox and a stable API kept forever, on three platforms. Downloadable Styles and Story elements (v3) give the Author more without running anyone's code, because they're files, not code.
- **The list of Mods in the Preamble.** The obvious place, next to the Work's own Story elements, but it puts Mods under Milestones and Reverts.
- **One switch per Vault, with Device overrides (like the other settings).** Simplest, but the Author wants Mods per Work: a short story has no use for a Chapter list.
