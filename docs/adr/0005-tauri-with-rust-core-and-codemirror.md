# Tauri 2 with a Rust core and a CodeMirror 6 interface

The app is built with Tauri 2. Typst, the History engine and SQLite live in a Rust core. The interface (CodeMirror 6 editor, Page preview, Vault browser) is web-based and runs in each platform's built-in web engine. The two hardest parts suit Rust: Typst is written in it, and the History engine must behave identically on every Device. CodeMirror 6 already provides the Formatted view (Obsidian is built on it), which no Kotlin or Flutter editor does.

## Considered Options

- **Kotlin + Compose Multiplatform.** Strong on Android, but there's no editor that can do the Formatted view, and it needs a bridge to Typst's Rust.
- **Flutter + Rust.** Draws identical UI on every platform, but its editors are weaker for hide/reveal markup.
- **Electron.** No Android support, and large.

## Consequences

- The interface must be tested on two web engine families: Chromium (Windows WebView2, Android WebView) and WebKit (Debian's WebKitGTK).
- Logic belongs in the Rust core. The web layer only displays and captures input, which keeps per-engine differences small.
- Typst draws the Page preview in Rust, so pages look the same on every Device even where the app's own interface differs slightly.
- Typing on Android (on-screen keyboards, autocorrect) is the main risk area and needs hand testing on real devices.
