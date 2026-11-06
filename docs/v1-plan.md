# v1 plan

What the first version contains, what waits for v2, and the order to build it in. Terms are defined in [CONTEXT.md](../CONTEXT.md). The reasoning behind the big decisions is in [docs/adr/](adr/).

## What v1 is

One **Author** can write a novel or short story on Windows, Debian and Android from a single Syncthing-synced **Vault**. Every **Change** is kept in a **History** that follows text through **Moves**. **Reviewer** **Comments** survive editing. Typst typesets the **Work** with a live **Page preview**, and there are PDF and plain `.docx` **Exports**.

## Scope

### In v1

**History engine (Rust core)**

- Every character knows which Change wrote it ([ADR 0003](adr/0003-character-level-history-with-moves.md)).
- Kinds of Change: typing burst, deletion, replaced selection, paste, Move, Revert, External edit, Comment actions, Preamble edits, Style and Story element definitions recorded, image inserted or replaced.
- A typing burst closes when you pause (about 5 s), the cursor leaves, you start a different kind of action, or after about 60 s of nonstop typing. These numbers are for tuning and don't affect correctness. Typos fixed inside an open burst are never recorded.
- A typing burst is one stretch of new text at one spot, anchored to the characters on either side, not to an index. While open it's a draft that can grow and shrink, kept in memory and the recovery file but never in the `.db`. Backspacing past its start or deleting past its end closes it and starts a deletion. Deleting is a burst too: backspacing or deleting forwards at one spot hides the text at once and records it as one deletion when it closes, by the same rules; typing closes it. Typing over a selection records one "replaced selection". A burst that ends up empty records nothing. Merges don't close a burst. Its Change ID is reserved when it opens, and its timestamp is set when it closes.
- Moves work on any selection, and edits made elsewhere to moved text follow it. Only text added strictly inside the moved range follows; text added at either edge stays where it was.
- Revert also reverts dependent later Changes by default. A Revert can itself be reverted. Returning to a Milestone reverts everything after it.
- Everything that isn't the Work's text (Preamble settings, Style and Story element definitions, Comment state, Reviewer and Device names, image slots) is stored as records with fields. Each field keeps its latest value in the fixed Change order, and older values stay in the History. Free text the Author edits outside the Work's text (the Preamble's raw Typst, and Comment bodies if they can be edited) is its own text sequence in the same engine, so concurrent edits to it merge instead of one being lost.
- A Milestone is exactly the set of Changes the naming Device had seen, stored as the highest counter seen from each Device. Returning to it is one Revert of every content Change that isn't part of it (later Reverts included, so the text is exactly as it was), and the Revert preview calls out Changes made before the Milestone on another Device but not part of it, so the Author can keep them. Device names, other Milestones and dismissed notices aren't content, so they stay. Renaming a Milestone is a field; deleting it is reverting it.
- A later Change depends on an earlier one only if it inserted text strictly inside text the earlier Change wrote (with that Change's characters on both sides), and so on transitively. Deletions and Moves are never dependents.
- A Change is in effect unless a Revert that is itself in effect targets it. "Restore this Change" reverts every in-effect Revert of it at once, so a Change reverted on two Devices offline comes back with one action.
- How Moves, positions and Change order work: [ADR 0008](adr/0008-moves-are-ranges-over-a-fugue-position-tree.md).
- One `.db` per Work. Every Device writes to it, and Conflict copies are merged back and deleted ([ADR 0002](adr/0002-one-self-healing-database-per-work.md)).
- Changes are stored as readable rows. A `.db` opened in DB Browser for SQLite makes sense to a person. Each Change row reads like a sentence: readable time, Device name, kind, and the text it inserted or deleted (deletions keep a copy of the deleted text, even though the engine could work it out). The engine's own columns sit next to these.
- Caches (the built tree, the current text, Session summaries) live in a cache folder on the Device, outside the Vault. They never sync and can always be rebuilt from the `.db`.
- The app never holds a `.db` open between writes. Unsaved Changes stay in a recovery file outside the Vault until the app has seen them in the `.db`.
- The Text file is regenerated on save. External edits are detected and recorded. The app recognises Text file contents it wrote itself, including in Conflict copies of the Text file.
- Each time the app writes a Text file, it records the file's hash and the version (the set of Changes) it was built from. A Text file with an unknown hash is compared against the text of the last version written there, and the differences become Changes anchored to that version, so they merge like Changes from an offline Device. If no written version can be found, the file is compared against the current text and the Author gets a Merge notice.
- In an External edit, a block of deleted text that reappears exactly somewhere else (at least a whole line, or about 40 characters) is recorded as a Move, so its History and Comments go with it; spaces at the block's edges may differ. Everything else is recorded as deletions and insertions. The file is compared word by word, so a changed word reads as one replacement, not as fragments of letters. All of one External edit's Changes form one Session, labelled "External edit". A byte order mark and Windows line endings added by another editor are ignored.
- Images are stored in the `.db` once per unique content, and written out next to the Text file only when the Work has any ([ADR 0007](adr/0007-images-live-inside-the-work.md)).
- `vault.db` holds Reviewers and shared settings, and merges the same way.

**Typst**

- Compiling runs in the Rust core. The Page preview supports clicking to jump to that spot in the text.
- On desktop, compile shortly after each typing burst. On Android, compile only while the Page preview is open.
- The Style library ships with default Styles, Looks and built-in Story elements. The Author can add Styles, Looks and their own Story elements.
- A Story element definition has a name, how it's written, a default look, how it appears in the Formatted view (chip or block), and what the `.docx` Export does with it.
- Built-in Story elements: Chapter, Part, scene break (typing `***` on an empty line converts it), emphasis, epigraph, block text (letters and in-world documents), dedication.
- The Preamble panel, collapsed by default, holds Looks, extra Styles, title, author name, language and raw Typst.
- Works record the Styles and Story element definitions they use, so Milestones re-render exactly as they were ([ADR 0004](adr/0004-works-record-styles-but-only-reference-fonts.md)).
- Fonts are looked up in the Vault's `fonts/` folder, then the Device's fonts, and otherwise the app warns and uses a default.

**Editor**

- CodeMirror 6 with a Formatted view (markup hidden except at the cursor, with hover tooltips on desktop) and a Raw view toggle.
- Story element chips, Reviewer-coloured Comment highlights, and optional "who/when" colouring by Session.
- The platform's built-in spellcheck can be switched on or off.

**Comments**

- Right-click to add a Comment, and pick the sender (the Author or a Reviewer).
- Reviewers belong to the Vault, each with a name, role label and colour.
- Comments are open or resolved. Resolved ones are hidden by default. Short replies are supported.
- Detached comments go to a side list with a quote of their original text, and reattach when the deletion is reverted.
- A Comment covers characters: the ones it was attached to, plus text later added strictly inside them (the same rule as Move ranges and Revert dependencies). If its text is split by a Move, the highlight shows in every part and "show me" steps through them. The Comments panel always keeps the original quote.

**History view**

- Sessions with word-level highlights and the number of words added.
- Named Milestones. Any two can be compared.
- "History of this text" from a selection, and Revert with a preview that lists dependent Changes.
- Merge notices, attached to the affected text, with a "show me" that jumps to each part. Every Device works them out from the History; only dismissing one is recorded, as a Change.

**App**

- On desktop: Vault browser and Chapter list on the left (dragging a Chapter is a Move), the editor with tabs in the middle, and a Page preview / Comments / History panel on the right.
- Quick switcher (Ctrl+O), focus mode, and word counts per Work and per Chapter.
- On the phone: one screen at a time, with a bottom bar to switch screens.
- Themes: light, dark or follow the system, plus CSS themes in the Vault's `themes/` folder. Which theme is used and the editor font size are Device settings.
- Device settings (theme, layout, Device ID, cursor positions) are never stored in the Vault.

**Exports**

- PDF in a chosen Look.
- Plain `.docx` in standard manuscript formatting, keeping only structure and emphasis. Story elements are handled as their definitions say.

**Platforms**

- Windows, Debian and Android.
- Android is a self-installed APK with "All files access" ([ADR 0006](adr/0006-android-uses-all-files-access.md)). All file access goes through one Rust module.

### Not in v1

Planned for v2:

- Collections
- Own spellchecker (Hunspell dictionaries through `spellbook`, plus a Vault-wide word list)
- AI comments over MCP: comments only, anchored by quote, run as a `writer mcp` mode
- Reading a second Work side by side
- Importing Word comments as Comments
- EPUB Export
- Play Store build using the folder picker
- Maybe: detect text that was both moved and edited in an External edit as a Move (fuzzy matching), if exact matching turns out not to be enough.
- Revisit which Changes count as dependent when reverting, based on how the v1 rule feels in real use.
- Smarter Move edges: text added at the edge of a moved range on another Device follows the Move when it clearly belongs to it. Example: if the range starts just after a ".", a word added after that "." (such as "Quietly, ") goes with the Move, and the same at the end. The rule must give the same answer on every Device even when the punctuation itself was edited concurrently.
- A cache for opening Works almost instantly, mainly for phones. The History engine's working state (the position tree, the layout, which Changes are applied and undone, the Moves, the clock) is saved as one file outside the Vault. Opening loads it and applies only the Changes added since. It needs no change to the engine's rules or to anything stored in the Vault:
  - Everything the engine shows is derived from the Changes, so the cache is only a shortcut. Deleting it loses nothing; the Work opens the normal way.
  - The tree and layout are flat arrays of numbers, so saving and loading them is close to copying memory.
  - Applying a batch of new Changes and recomputing once already handles Changes that sort before ones already applied, such as those from a merged Conflict copy.
  - The cache records the highest Change counter it holds from each Device, plus the total number of Changes. Anything that doesn't match the `.db` means the cache is discarded.
  - The cache file has a version number, so any change to the engine's in-memory shape (such as moving to FugueMax) just discards old caches.
  - Tested like the engine itself: opening from the cache plus new Changes must give exactly the same result as opening from scratch.
  - Until then, keep the `.db` the single source of truth. Nothing the engine needs may exist only in memory or only in a cache.

Out of scope entirely: notes and worldbuilding, grammar checking, Reviewers using the app themselves, and suggested edits.

## Build order

Riskiest first. Each phase ends with a "done when" check.

### Phase 0: Experiments (go/no-go)

Throwaway code, written to find showstoppers before investing in the real thing.

- **0a, typing on Android.** A Tauri 2 Android app with a CodeMirror 6 editor.
  *Done when:* typing, autocorrect, swipe typing and backspace work with Gboard and SwiftKey on a real phone, without lost or doubled characters.
  *Result (2026-11-04):* passed with Gboard on a Pixel 7. SwiftKey not tested; the app is private and Gboard is the keyboard in use.
- **0b, Typst on Android.** Compile a sample novel of about 300 pages from Rust inside the app, using fonts from a Vault folder.
  *Done when:* recompiling after a small edit feels instant (well under a second) on a mid-range phone.
  *Result (2026-11-05):* passed with Typst 0.15.1 on a generated 315-page novel. On the Pixel 7, setup and first compile took about 690 ms and recompiles 70–80 ms. On desktop, about 600 ms and 25–40 ms, with PDF export about 100 ms. Lessons for the real app:
  - **Evict Typst's cache after every compile** (`comemo::evict(1)`). Without it, memory grows by about 70 MB per edit on a novel this size, until Android kills the app for running out of memory. With `evict(1)` it stays around 200 MB, and recompiles get no slower.
  - Keep one `World` alive between compiles and only update its text. That's what makes recompiles incremental.
  - Build libraries optimised even in dev builds (`[profile.dev.package."*"] opt-level = 3`), or Typst runs 10–50 times slower and timings are meaningless.
  - Only one compile should run at a time. Take the lock before compiling, not after.
- **0c, files on Android.** With "All files access", open, write and close a SQLite file in the Syncthing folder while Syncthing syncs it from another Device.
  *Done when:* there's no corruption, and the app notices when the file was replaced underneath it.
  *Result (2026-11-05):* passed on a Pixel 7 and Windows, syncing through Syncthing: normal sync both ways, offline writes on both sides (clean Conflict copy), rapid simultaneous writes, closed app, and connection lost mid-sync. Every file reported `integrity: ok`, and the app noticed every change made underneath it. Lessons for the real app:
  - Syncthing must ignore `*-journal` on every Device, so SQLite's temporary journal file never syncs. Ignore patterns aren't synced themselves, so the app should set this up when a Vault is opened.
  - Android never shows a pop-up for "All files access", and can lock the switch for apps that weren't installed from a store. The app has to detect the missing permission and open the settings page itself.
  - App IDs must contain no underscores or hyphens (Windows rejects underscores, and Android package names can't contain hyphens).

If 0a fails, revisit [ADR 0005](adr/0005-tauri-with-rust-core-and-codemirror.md) before going further.

### Phase 1: History engine

A standalone Rust library with no UI. Everything else is built on it.

- Data model: Changes as rows, character identity, Moves, Reverts, Sessions, Milestones.
- Merging: take every Change from both copies, and absorb Conflict copies.
- Text file generation, and importing External edits (compare the file with the current text and record the difference as Changes).
- The burst interface the editor will call: open a burst, extend it, close it.
- A terminal harness to try it out by hand (the `/prototype` skill fits here).
- A stress harness: a fixed-seed generator writes realistic Histories at increasing sizes (mostly typing near the last edit, some deletions, few Moves and Reverts) and times opening the Work and single edits on the full text. Speed-ups that update state step by step are checked against recomputing everything from scratch, which stays as the reference.

*Done when:*

- Three simulated Devices making random offline edits, Moves and Reverts, merged in any order, always end with identical text (property tests).
- A Work's `.db` opened in DB Browser for SQLite reads as plain, understandable rows.
- A generated novel-sized History (about 30,000 Changes) loads in under a second on desktop.
- On that History, a keystroke takes under a frame (about 16 ms) on desktop.

Reading: Weidner & Kleppmann, *The Art of the Fugue: Minimizing Interleaving in Collaborative Text Editing* (2023); Kleppmann, *Moving Elements in List CRDTs* (2020).

### Phase 2: Desktop editor

- A Tauri 2 app: Vault browser, opening a Work, and CodeMirror connected to the History. Typing bursts become Changes, and Changes merged from Conflict copies or External edits appear in the editor.
- Raw view first, then the Formatted view.
- Tabs, quick switcher, Device settings, recovery file.

*Done when:* you can write a short story on Windows and Debian, edit its Text file in vim, sync both machines through Syncthing with offline edits on each, and nothing is lost.

### Phase 3: Typst

- Compiling from the History, the Page preview, and click-to-jump.
- The Style library, default Looks, built-in and Author-defined Story elements, and the Preamble panel.
- Styles and Story element definitions recorded in the History, and font lookup.
- PDF and `.docx` Exports.

*Done when:* the same Work produces an identical PDF on Windows and Debian with the same Vault fonts, an old Milestone re-renders unchanged after a Style is edited, and the `.docx` opens cleanly in Word and LibreOffice.

### Phase 4: Comments

- Reviewers in `vault.db`, plus adding, replying to and resolving Comments, coloured highlights, and Detached comments.

*Done when:* a Comment survives a Move, a partial deletion, a full deletion (it detaches) and a Revert (it reattaches).

### Phase 5: History view

- Sessions with word-level highlights, Milestones and comparing them, "History of this text", Revert with preview, and returning to a Milestone.

*Done when:* in the "The rain had stopped." → "eased" scenario, reverting the first Change removes the sentence cleanly by default, and leaves "eased" behind only when you ask for that.

### Phase 6: Polish and Android

- The Chapter list with drag-to-reorder, focus mode, word counts, themes, and the spellcheck switch.
- The Android layout (one screen at a time), the "All files access" flow (detect the missing permission and open its settings page; see the 0c lessons), and compiling only while the Page preview is open.
- Testing: Playwright on Chromium and WebKit, plus hand testing on a real phone with real keyboards.

*Done when:* a full novel goes back and forth between Android, Windows and Debian through Syncthing, with offline edits on all three, and ends with identical text and Comments everywhere.

## Open design questions for Phase 1

Settle these while building the History engine, and write an ADR for any answer that's hard to reverse.

- ~~Two Devices Move the same text to different places while offline.~~ Settled: the later Move in the fixed Change order wins (so it depends on how Change IDs are formed).
- ~~One Device Moves some text while another deletes part of it, or Moves part of it somewhere else.~~ Settled: a deletion wins wherever the text went. With overlapping Moves, only the overlap goes with the winning Move, the rest of each Move still happens, and the Author gets a Merge notice.
- ~~How a Change ID is formed and how a Device gets its ID.~~ Settled: the ID is Device ID + counter. Changes are ordered by a hybrid logical clock (wall-clock time that never goes below anything already seen), then by Device ID. A Device ID is random and short, made on first launch, kept as a Device setting and left out of Android backups. The History records a readable name for each Device. A merge that finds the same ID with different content refuses loudly.
- ~~How long a gap ends a Session.~~ Settled: each Change stores its Session ID (the ID of the Session's first Change). A new Session starts when the Work is opened on a Device, or after about 30 minutes without a Change (to be tuned in use). A Session never spans Devices or Works.
- How the editor shows Changes merged in from elsewhere without making your cursor jump. Partly settled: an open typing burst is anchored to characters, so merges don't move it. Cursors and selections outside a burst are still open (Phase 2).
- A Style file in `styles/` edited on two Devices. Style files are plain files with no History, so their Conflict copies can't merge automatically. Proposal: show both versions and let the Author pick.
- ~~An External edit on one Device while the app edits the same Work on another.~~ Settled: the External edit is compared against the version the app last wrote to that Text file, not against the current text, and then merges as normal.

## Main risks

| Risk | Mitigation |
|---|---|
| Custom merge logic with Moves is hard to get right | Build it alone in Phase 1, with property tests, before any UI |
| Android keyboards and web editors don't mix well | Phase 0a go/no-go; hand testing on real devices |
| WebKitGTK on Debian (speed, graphics glitches) | Use only well-established web features; test on WebKit with Playwright; known workaround flag for graphics issues |
| Syncthing replacing a `.db` while it's being written | Never hold it open; keep Changes until they're seen in the file (Phase 0c) |

## Reference: Vault layout

```
Writing/                        ← the Vault (Syncthing folder)
├── The Long Night.db           ← a Work's History
├── The Long Night.typ          ← its Text file (regenerated)
├── The Long Night.images/      ← regenerated image copies, only if the Work has images
├── Short stories/              ← ordinary folders, any arrangement
│   ├── Rain.db
│   └── Rain.typ
├── styles/                     ← Style library: Styles, Looks, Story elements
├── themes/                     ← app themes (CSS)
├── fonts/
└── vault.db                    ← Reviewers and shared settings
```
