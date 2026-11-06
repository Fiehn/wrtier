# Building and installing on Debian

Tauri can't build the Linux app from Windows, so it's built on the Debian machine itself and installed as a `.deb`. This was written for Debian 12 (bookworm) and 13 (trixie).

## 1. Install what the build needs (once)

The system libraries Tauri uses (WebKitGTK is the web engine the app runs in on Linux):

```sh
sudo apt update
sudo apt install build-essential pkg-config curl wget file git \
  libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

**Rust**, from rustup (Debian's own `rustc` is too old for the 2024 edition the code uses):

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

**Node.js 22 or newer** (the editor's build tool needs at least 20.19; Debian 12's packaged Node is older). With nvm:

```sh
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash
source ~/.bashrc
nvm install 22
```

## 2. Build the `.deb`

```sh
git clone <the repository> wrtier
cd wrtier/app
npm ci
npm run tauri build -- --bundles deb
```

The release build is optimised for size and speed (link-time optimisation), so it takes a few minutes the first time. If the machine runs short of memory, limit the parallel jobs: `CARGO_BUILD_JOBS=2 npm run tauri build -- --bundles deb`.

The package ends up in the workspace's `target` folder, next to `app/`:

```sh
ls ../target/release/bundle/deb/
```

## 3. Install it

```sh
sudo apt install ../target/release/bundle/deb/Wrtier_0.1.0_amd64.deb
```

`apt` (rather than `dpkg -i`) also installs the WebKitGTK runtime it depends on. Wrtier then has an entry in the applications menu.

**To update** later: `git pull`, then steps 2 and 3 again.

## If the window is blank, flickers or draws wrongly

WebKitGTK's hardware rendering doesn't get along with some graphics drivers (often NVIDIA, sometimes under Wayland). Start it once from a terminal with the renderer workaround:

```sh
WEBKIT_DISABLE_DMABUF_RENDERER=1 wrtier_app
```

If that's not enough, also try `WEBKIT_DISABLE_COMPOSITING_MODE=1`. When one of them helps, make it permanent by adding the line to `~/.profile`, for example `export WEBKIT_DISABLE_DMABUF_RENDERER=1`, and logging in again.

(The installed program may be called `wrtier` instead of `wrtier_app`, depending on the Tauri version: `dpkg -L wrtier | grep bin/` shows it.)

## Where Wrtier keeps things on this Device

- Device state (the Device ID, open tabs, window size): `~/.local/share/com.fiehn.wrtier/device.json`
- Recovery files: `~/.local/share/com.fiehn.wrtier/recovery/`

Neither is ever in the Vault. Don't copy `device.json` from another computer: two Devices with the same ID can't merge their Histories (the merge refuses and reports a Sync problem).

## Phase 2's final test

*Done when:* you can write a short story on Windows and Debian, edit its Text file in vim, sync both machines through Syncthing with offline edits on each, and nothing is lost.

1. **Set up.** Sync the Vault folder to Debian with Syncthing. Open it in Wrtier on Debian and give the Device a name (say "Debian"). The app adds `*-journal` to the folder's `.stignore`.
2. **Online, one at a time.** On Windows, make a Work "Rain" and write a paragraph. Within seconds of Syncthing syncing, Rain shows up in Debian's Vault browser; open it, and the text is the same.
3. **Online, both open.** With Rain open on both, type on Windows. Debian takes it in within a few seconds ("Took in changes from …") without moving Debian's cursor. Then the other way round.
4. **Offline on both.** Pause Syncthing on both machines (or take one off the network).
   - On Windows, write a new paragraph at the end of Rain.
   - On Debian, change a sentence in the middle. Then close Rain, and in vim change a word in `Rain.typ` (leave the last `// Wrtier version:` line as it is) and save.
5. **Back online.** Resume Syncthing. It makes Conflict copies; the app merges them and deletes them (open the Vault browser on each machine, or open Rain).
6. **Check that nothing was lost.** On both machines, Rain has the Windows paragraph, the Debian sentence and the vim word.
   - The Text files should be identical once both have written them (switch away from the window or close Rain on each, then compare): `sha256sum Rain.typ` on Debian and `Get-FileHash Rain.typ` on Windows print the same hash.
   - No Sync problems in the Vault browser, and no `.sync-conflict-` files left in the Vault.
7. **A crash, while you're at it.** On Debian, type a sentence and within a few seconds kill the app (`pkill -i wrtier`). Start it again: the sentence is back, with a "Recovered … characters" line.

If any step loses text, keep the Vault as it is (don't fix it by hand): the `.db` files and any Conflict copies show what happened.
