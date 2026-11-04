# One self-healing database file per work

Each work's History lives in a single SQLite file in the vault, and every device writes to it directly. When offline edits on two devices make the sync tool produce a conflict copy, the next device to open the vault merges it back and deletes it. Merging is just taking every Change from both files, which is safe because Changes are append-only and uniquely identified. We chose this because the author wants one plain, readable file per work that they own and can hand to someone as-is, with every device counting equally.

## Considered Options

- **Per-device log files, merged in memory.** These never conflict, but they leave a folder of files the author has to zip before sharing.
- **A designated "home device" that merges everything into one file.** Rejected because no device should be in charge.

## Consequences

- Changes must always merge automatically without asking the author, so the History's data model has to give the same result regardless of the order Changes arrive in.
- Changes are stored as readable rows, not as an opaque binary format from a library.
- The app never holds the file open between writes, and keeps unsaved Changes elsewhere until it has seen them in the file.
- This relies on the sync tool keeping conflict copies, which is Syncthing's default.
