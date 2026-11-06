# Text files name the version they were written from

Every Text file the app writes ends with one Typst comment line, `// Wrtier version: 7f8c3ffd:120, 3e91a2b0:48`: the highest Change counter it held from each Device. Syncthing may deliver another Device's Text file before that Device's History, and without the line the app can't tell that file from an External edit, so it would record the other Device's writing a second time. With it, a Text file written from Changes the History doesn't have yet simply waits until they arrive, and an edit made in another editor is compared against exactly the version it started from. A file whose line is missing or edited is read the old way: by its hash, then against the version last written.

The line exists only in the file on disk. It's added when the file is written and removed when it's read, so it's never part of the Work's text, and Typst ignores it.

## Considered Options

- **Wait until things settle** (import a Text file with an unknown hash only after nothing has changed for ~20 seconds). Keeps the file plain, but it's a guess a slow sync can still beat, and every edit made in vim shows up late.
