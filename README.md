<p align="center">
  <img src="app/src-tauri/icons/source/wrtier_with_text.svg" alt="Wrtier logo" width="600">
</p>

<h1 align="center"></h1>

# Goals
wrtier is a **cross device locally owned history aware writing program**. It is a writing instrument that is built to allow unobstructed writing in a native desktop app, while also allowing cross device sharing and a fully reversible history of all changes made in a work. It was built initially to avoid the use of Google Docs as a tool for cross device writing due to uncertainty pertaining to theft (for the training of LLMs) and general insecurity in the terms of service regarding user data ([Recent changes to photos and G-mail](https://www.androidauthority.com/google-personal-intelligence-privacy-3727111/9) and [many more coming](https://policies.google.com/privacy)). For a deeper dive read The Age of Surveillance Capitalism[1].

The wrtier program is built on four principles.
1. History.

All changes of any work you create are stored as an immutable list, when you make any change it is recorded and kept for later viewing or reverting. This history allows for a full walkthrough of changes and clear comparison to earlier drafts. It is built using a human readable database of changes.

2. Simplicity

All elements of the program are stored as human readable, no unparseable binaries and no zip files of hundreds of documents. Each work contains two files. The database of changes and a text (``.typ``)  file that holds the latest final text including compilation instructions for look and styling using [typst](https://typst.app/) compilation. All other information relates to the look, feel and functionality of the editor and not the actual work. This allows easy disentanglement from the program at any point in time without ever opening the program again.

3. Ownership

All files are yours and never shared outside who you decide to share it with. The cross device part of the program is entirely provided by a separate installation of [Syncthing](https://syncthing.net/) and the apps natural integration with that program once installed. 

4. Cross Device

The cross device nature of the program relies on an optional installation of [Syncthing](https://syncthing.net/). From that the app will handle sync failures and merging as gracefully as possible but never remove anything if failing. The program's part is handling how [Syncthing](https://syncthing.net/) works and ensuring that the writing is smooth. Simultaneous editing is possible due to the nature of the program, the [fugue algorithm](https://arxiv.org/abs/2305.00583) and graceful merging but will always be slightly behind and it is not an intended function and thus can result in undesired behaviour. The program will attempt to create a similar experience and byte similar exports. There will be some differences in the look and behavior of the program as [Tauri](https://v2.tauri.app/) uses the native browser viewer.

# Getting Started
Currently there are two ways to get the program. Firstly the [release page](https://github.com/Fiehn/Wrtier/releases) which include executables for debian, windows and Android APK. This gives you the latest stable and tested release. Otherwise you can build the program yourself using [build](#building) guide. This should also allow for building on MacOS, but no guarantees of this are given.

If you want cross device acces you need to install [Syncthing](https://syncthing.net/) on all devices desired. Then you can use the program to set it up or you can follow [Syncthings own guide](https://docs.syncthing.net/intro/getting-started.html) and set up a shared folder. This is usually quick and easy.

# Getting in Touch or Suggestions
If you find a bug or have suggestions for features you are welcome to report it in the [Github Issue Tracker](https://github.com/Fiehn/Wrtier/issues). Otherwise please don't I guess :)

# Building
You need [Rust](https://rustup.rs), [Node.js](https://nodejs.org) 22 or newer, the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your system, and cargo-about (it writes the app's list of licenses):

```sh
cargo install cargo-about --locked --features cli
```

Then build from the `app` folder:

```sh
cd app
npm ci
npm run tauri build
```

The program is written to `target/release/` at the root of the repository, and installers to `target/release/bundle/`. On Windows the program runs on its own without installing. For Debian, see [docs/debian.md](docs/debian.md). For Android, run `npm run tauri android init` once, then `npm run tauri android build` (this needs the Android SDK and NDK from Android Studio).

# License
This program is provided under the [Apache 2.0](LICENSE) license and includes a license list of all dependencies.

# References
[1] Zuboff, Shoshana (2019). The Age of Surveillance Capitalism: The Fight for a Human Future at the New Frontier of Power.
