# Typst instead of LaTeX

Manuscripts and styles are written in Typst, and the Typst compiler is embedded in the app as a library. LaTeX was the author's existing tool, but it has no real incremental compilation (so no live preview on every change), a TeX distribution can't be cleanly embedded in an Android app, and its output varies with each machine's installed packages and fonts. Typst recompiles incrementally, ships as one small Rust library on Windows, Debian and Android, and produces identical output everywhere when the fonts are bundled in the vault.
