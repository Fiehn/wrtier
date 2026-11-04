# Works record the Styles they use, but only reference fonts

Each Work's History stores the content of every Style the Work uses. Whenever a Style in the library changes, the Work records that as a new Change, so every Milestone re-renders exactly as it was and a Work's `.db` compiles on its own. Fonts are the exception: they are large and often licensed, so the History records only font names. The app looks for each font first in the Vault's `fonts/` folder, then among the Device's installed fonts, and otherwise warns and falls back to a default font.

## Considered Options

- **Styles as plain files with no history.** Old Milestones would render with today's Styles, and a `.db` couldn't compile without the rest of the Vault.
- **Embedding font files in each `.db`.** Too large and often not allowed by font licences.
