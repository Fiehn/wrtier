# v1 plan

What the first version contains, what waits for v2, and the order to build it in. Terms are defined in [CONTEXT.md](../CONTEXT.md). The reasoning behind the big decisions is in [docs/adr/](adr/).

## What v1 is

One **Author** can write a novel or short story on Windows, Debian and Android from a single Syncthing-synced **Vault**. Every **Change** is kept in a **History** that follows text through **Moves**. **Reviewer** **Comments** survive editing. Typst typesets the **Work** with a live **Page preview**, and there are PDF and plain `.docx` **Exports**.

## Scope

### In v1

**History engine (Rust core)**

- Every character knows which Change wrote it ([ADR 0003](adr/0003-character-level-history-with-moves.md)).
- Kinds of Change: typing burst, deletion, replaced selection, paste, Move, Revert, External edit, Comment actions, Preamble edits, Style and Story element definitions recorded.
- A typing burst closes when you pause (about 5 s), the cursor leaves, you start a different kind of action, or after about 60 s of nonstop typing. These numbers are for tuning and don't affect correctness. Typos fixed inside an open burst are never recorded.
- A typing burst is one stretch of new text at one spot, anchored to the characters on either side, not to an index. While open it's a draft that can grow and shrink, kept in memory and the recovery file but never in the `.db`. Backspacing past its start or deleting past its end closes it and starts a deletion. Deleting is a burst too: backspacing or deleting forwards at one spot hides the text at once and records it as one deletion when it closes, by the same rules; typing closes it. Typing over a selection records one "replaced selection". A burst that ends up empty records nothing. Merges don't close a burst. Its Change ID is reserved when it opens, and its timestamp is set when it closes.
- Moves work on any selection, and edits made elsewhere to moved text follow it. Only text added strictly inside the moved range follows; text added at either edge stays where it was.
- Revert also reverts dependent later Changes by default. A Revert can itself be reverted. Returning to a Milestone reverts everything after it.
- Everything that isn't the Work's text (Preamble settings, Style and Story element definitions, Comment state, Reviewer and Device names) is stored as records with fields. Each field keeps its latest value in the fixed Change order, and older values stay in the History. Free text the Author edits outside the Work's text (the Preamble's raw Typst, and Comment bodies if they can be edited) is also a single field in v1. When two Devices set it without seeing each other's edit, the later one wins and the Author gets a Merge notice whose "show me" opens the other version, so it can be copied back.
- A Milestone is exactly the set of Changes the naming Device had seen, stored as the highest counter seen from each Device. Returning to it is one Revert of every content Change that isn't part of it (later Reverts included, so the text is exactly as it was), and the Revert preview calls out Changes made before the Milestone on another Device but not part of it, so the Author can keep them. Device names, other Milestones and dismissed notices aren't content, so they stay. Renaming a Milestone is a field; deleting it is reverting it.
- A later Change depends on an earlier one only if it inserted text strictly inside text the earlier Change wrote (with that Change's characters on both sides), and so on transitively. Deletions and Moves are never dependents.
- A Change is in effect unless a Revert that is itself in effect targets it. "Restore this Change" reverts every in-effect Revert of it at once, so a Change reverted on two Devices offline comes back with one action.
- How Moves, positions and Change order work: [ADR 0008](adr/0008-moves-are-ranges-over-a-fugue-position-tree.md).
- One `.db` per Work. Every Device writes to it, and Conflict copies are merged back and deleted ([ADR 0002](adr/0002-one-self-healing-database-per-work.md)). History copies are merged first; then Text file copies are read like External edits (a copy another Device wrote is recognised and simply deleted). A copy that can't be absorbed (not a History, written by a newer version of the app, two Devices sharing a Device ID, or damaged) is never deleted: it's kept and reported as a Sync problem. Conflict copies are absorbed as soon as the app sees them (when the Vault opens, or when one appears), for every Work in the Vault, open or not. A copy whose main file is missing holds a whole History, so it becomes the main file.
- Changes are stored as readable rows. A `.db` opened in DB Browser for SQLite makes sense to a person. Each Change row reads like a sentence: readable time, Device name, kind, and the text it inserted or deleted (deletions keep a copy of the deleted text, even though the engine could work it out). The engine's own columns sit next to these.
- The app shows a Device's current name from `vault.db`, then the name the Work recorded, then its ID. Just before a Device's first edit to a Work in a sitting (before any typing burst opens, since nothing else may be recorded while one is open), it records its current name in that Work if the Work has none or an older one. Reading never writes, and a rename reaches each Work the next time it's edited.
- Caches (the built tree, the current text, Session summaries) live in a cache folder on the Device, outside the Vault. They never sync and can always be rebuilt from the `.db`.
- The app never holds a `.db` open between writes. Unsaved Changes stay in a recovery file outside the Vault until the app has seen them in the `.db`.
- The Text file is regenerated on save. It starts with a generated header holding the Work's recorded Styles, Looks and Story elements, so it compiles on its own in the Work's first Look; the header is never part of the Work's text, and edits to it outside the app are dropped ([ADR 0011](adr/0011-text-files-carry-a-generated-header.md)). External edits are detected and recorded. The app recognises Text file contents it wrote itself, including in Conflict copies of the Text file.
- Each time the app writes a Text file, it records the file's hash and the version (the set of Changes) it was built from. A Text file with an unknown hash is compared against the text of the last version written there, and the differences become Changes anchored to that version, so they merge like Changes from an offline Device. If no written version can be found, the file is compared against the current text and the Author gets a Merge notice.
- In an External edit, a block of deleted text that reappears exactly somewhere else (at least a whole line, or about 40 characters) is recorded as a Move, so its History and Comments go with it; spaces at the block's edges may differ. Everything else is recorded as deletions and insertions. The file is compared word by word, so a changed word reads as one replacement, not as fragments of letters. All of one External edit's Changes form one Session, labelled "External edit". A byte order mark and Windows line endings added by another editor are ignored.
- `vault.db` holds the Vault's Devices and their names, Reviewers, and Vault settings (including each Device's overrides), and merges the same way. A folder is a Vault because it has a `vault.db`; picking a folder without one asks before making it a Vault.

**Typst**

- Compiling runs in the Rust core. The Page preview supports clicking to jump to that spot in the text.
- Compile only while the Page preview is open, on every platform, about 300 ms after the last keystroke. It compiles the text as it is on screen, including an open typing burst; compiling reads the History and never writes to it. Exports always compile, with the preview open or not.
- When the Work doesn't compile, the Page preview keeps the last pages that did, faded, under a bar with the first error and "Show me" (which jumps to it like click-to-jump). Errors appear only after the same pause as compiling, so they don't flicker while typing. An error in a Style, Look or Story element names the file in the Style library it comes from. A PDF Export refuses to run while there's an error; Typst warnings never block anything and show as a count in the bar. The `.docx` Export doesn't use the compile, so Typst errors don't block it.
- The Page preview keeps the cursor's page in view. Whether it follows the cursor is a Vault setting (a Device can override it), and it's on by default.
- The Style library ships with default Styles, Looks and built-in Story elements. The Author can add Styles, Looks and their own Story elements.
- The defaults are written into `styles/` when the Vault is made and then belong to the Vault, so every Device reads the same ones whatever its app version. A newer app never overwrites them: defaults the Vault has never had are added once (`vault.db` records which were written, so deleted ones don't come back), and improved versions of existing ones are only offered, with Compare / Use it ([ADR 0012](adr/0012-default-styles-are-copied-into-the-vault.md)).
- A Story element definition has a name, how it's written, a default look, how it appears in the Formatted view (chip or block), and what the `.docx` Export does with it.
- Built-in Story elements: Chapter, Part, scene break (typing `***` on an empty line converts it), emphasis, epigraph, block text (letters and in-world documents), dedication.
- The Preamble dialog holds the whole Preamble: Looks (added, removed and reordered; the first is the Work's default), extra Styles, the Author's own Story elements, title, author name, language and raw Typst. It opens from a "Preamble…" button on the Work in the Vault browser (next to Rename and Trash), from the Work's right-click menu, and from a shortcut while writing. It works on a Work that isn't open too: the change is recorded in its History like any other. There is no Preamble panel in the editor.
- Style library files open in an ordinary app tab (Raw view with Typst highlighting, on every platform). They have no History of their own and are saved on a pause, like the Text file. "New Story element" is a small form (name, chip or block, `.docx` behaviour) that writes the file with a correct header and a starter function that labels its output (`<name>`), so Styles can restyle it at once. "New Style" and "New Look" create the file and open it. An open Work notices a library change from anywhere within about a second: the Page preview recompiles and the Style update notice appears.
- The built-in Story elements are part of every Work. The Author's own are added to a Work explicitly in the Preamble dialog, which is when the Work records their definition. Using one that isn't added (e.g. `#dream[…]`) shows an error chip in the Formatted view with "dream isn't in this Work. Add it?", and the Text file's header only carries what the Work added.
- Works record the Styles and Story element definitions they use, so Milestones re-render exactly as they were ([ADR 0004](adr/0004-works-record-styles-but-only-reference-fonts.md)).
- Fonts are looked up in the Vault's `fonts/` folder, then the Device's fonts, and otherwise the app warns and uses a default. The default is Typst's own fonts (`typst-assets`), bundled in the app, so it's the same on every Device. A Device font is only used for a family neither the Vault nor the app has, so a Device's own copy of, say, Libertinus never changes how the default looks. A font that comes from the Device instead of the Vault is flagged in the Page preview and at Export ("other Devices may render this Work differently"), with a hint to copy it into `fonts/`.
- A PDF's date is the time of the latest Change in the version rendered, never the current time, so exporting the same version twice, on any Device, gives a byte-identical file.

**Editor**

- CodeMirror 6 with a Formatted view (markup hidden except at the cursor, with hover tooltips on desktop) and a Raw view toggle.
- Typing `#` lists what can go there (completions), narrowing as you type: the Work's Story elements first, then Typst's own functions.
- In the Formatted view a Story element's brackets are never shown: its command shows as a label (and as editable `#name` while the cursor is in it), and a block's lines are set apart. The brackets can't be deleted on their own; deleting the command's `#` (at once or letter by letter) removes both brackets with it and keeps the text. The Raw view shows and edits everything as written.
- The Formatted view gets its markup positions from a small parser in the editor for the prose part of Typst (headings, emphasis, strong, comments, escapes, line breaks, function calls), so formatting keeps up with every keystroke. Where it disagrees with Typst it only shows raw markup; compiling always uses Typst itself. If that happens too often, Typst's own parser compiled to WebAssembly can take its place, since both only give the editor markup positions.
- Story element chips, Reviewer-coloured Comment highlights, and optional "who/when" colouring by Session.
- The platform's built-in spellcheck can be switched on or off.
- Moving text in the editor: dragging a selection is a Move, and so is cutting text and then pasting exactly that text in the same Work. Until the paste, the cut text is only hidden (like a deletion burst, but it doesn't close on a pause); any other edit, pasting different text, pasting in another Work or closing the Work records the cut as a deletion. Keyboard shortcuts for moving paragraphs come in Phase 6. (Tauri's own drag-and-drop handling is off for the window, `dragDropEnabled: false`: on Windows it swallows the web view's drag and drop, so text couldn't be dragged. Dropping files onto the app would need it handled another way.)
- Ctrl+B and Ctrl+I type `*…*` and `_…_` around the selection (widened to whole words, since Typst ignores them inside a word), take them off text that already has them, or type a pair to type into. The words keep their History: only the markers are inserted.
- Ctrl+Z: while a typing burst is open, undo goes step by step inside it (nothing there is recorded yet), and its undo steps are cleared when it closes. After that, Ctrl+Z reverts this Device's latest Change still in effect and Ctrl+Shift+Z restores it, so undone and redone text keeps its History and Comments.

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
- The Vault browser lists every History in the Vault except `vault.db`, plus Text files without one, which become Works when opened. It hides a file only when it's sure the file isn't a Work (some other program's database). A History from a newer version of the app, or a file that can't be read right now, is listed with the reason and is a Sync problem. Conflict copies are never listed. Whether a file is a History is decided in one place in the engine, next to the storage version.
- Deleting a Work moves its `.db`, `.typ` and `.images/` into a `.trash/` folder in the Vault (hidden from the Vault browser, but synced), so it can be restored from any Device, Android included. The trash is emptied only by hand.
- Renaming a Work in the app renames its `.db`, `.typ` and `.images/` together.
- A Work open on one Device that's renamed or trashed on another is followed, never recreated at its old path: the tab finds its History again by the Work's ID, a random ID recorded in the History when the Work is made (a Work from before that is known by its first Change together with that Change's time; the Change ID alone isn't enough, since Change IDs count up per Work and every Work a Device makes starts with the same one). A tab only moves onto a History that has everything it saved, and saving refuses an ID the file already has for a different Change, so two copies of a History that went their own ways are never mixed. Renamed: the tab moves along, with a line saying so. In `.trash/`: saving pauses, with Restore or Close. Gone entirely: saving pauses, with "Save what's here as a new Work" or Close. Nothing typed is lost meanwhile; the recovery file holds it. A Text file without a History whose exact contents some History in the Vault recorded writing is a leftover from a Work renamed outside the app: it's listed as "left over from" that Work, with an option to delete it, never offered as a new Work.
- Quick switcher (Ctrl+O), focus mode, and word counts per Work and per Chapter.
- On the phone: one screen at a time, with a bottom bar to switch screens.
- Themes: light, dark or follow the system, plus CSS themes in the Vault's `themes/` folder. Which theme is used and the editor font size are Vault settings, and a Device can override any of them for itself.
- Device state (Device ID, the last Vault opened, window layout, open tabs, cursor positions) is never stored in the Vault. A Device finds its name and overrides in `vault.db` by its Device ID; one that isn't there yet asks for a name.

**Exports**

- PDF in a chosen Look.
- Plain `.docx` in standard manuscript formatting, keeping only structure and emphasis. Story elements are handled as their definitions say.
- The `.docx` is built by the app itself: it reads the Work's text with Typst's own parser (`typst-syntax`) and writes the file with a small Rust crate, so it works on Android and never depends on a Look. Emphasis becomes italics. Each Story element definition picks one behaviour from a fixed menu: Chapter heading (new page), Part heading (new page, centred), centred `#` line, indented block, centred paragraph, plain text, or left out. Anything else (such as raw Typst code in the text) is skipped and listed in a short report. Fixed manuscript format: 12 pt Times New Roman (Courier as an option), double-spaced, 1-inch margins, first-line indents, header with surname / title / page number.

**Platforms**

- Windows, Debian and Android.
- Android is a self-installed APK with "All files access" ([ADR 0006](adr/0006-android-uses-all-files-access.md)). All file access goes through one Rust module.

### Not in v1

Planned for v2:

- Images, as [ADR 0007](adr/0007-images-live-inside-the-work.md) describes: stored in the `.db` once per unique content, inserting or replacing one is a Change, and copies are written to `<Work>.images/` next to the Text file only when the Work has any. Written in the text as `#image("map.png")`, where the name is an image slot in the Work, so Milestones show the image the slot had then. Not needed for v1: novels rarely use images, and v1 is for getting real use out of the app first. Renaming and trashing a Work already move `.images/` along, so nothing else changes when they arrive.
- Collections
- Own spellchecker (Hunspell dictionaries through `spellbook`, plus a Vault-wide word list)
- AI comments over MCP: comments only, anchored by quote, run as a `writer mcp` mode
- Reading a second Work side by side
- Importing Word comments as Comments
- EPUB Export
- A richer `.docx` Export: Styles and Story element definitions describe their own `.docx` formatting in their header comment, instead of picking from v1's fixed menu. Design it once it's clearer what `.docx` can express.
- Play Store build using the folder picker
- Maybe: detect text that was both moved and edited in an External edit as a Move (fuzzy matching), if exact matching turns out not to be enough.
- Free text outside the Work's text (the Preamble's raw Typst, Comment bodies) as its own text sequence in the History engine, so concurrent edits merge instead of the later one winning. Stored data doesn't change: old field values stay readable, and new edits start using the sequence.
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

Phase 1 is officially done. No terminal harness is needed.

### Phase 2: Desktop editor

- A Tauri 2 app: Vault browser, opening a Work, and CodeMirror connected to the History. Typing bursts become Changes, and Changes merged from Conflict copies or External edits appear in the editor.
- Raw view first, then the Formatted view.
- Tabs, quick switcher, Device state, Vault settings, recovery file.
- Saving: the `.db` is written every time a typing burst closes. The Text file is written at most every 30 seconds while writing, and whenever the Author leaves the Work (another tab, closing it, the window losing focus, quitting), so it's current whenever another program might read it.
- The recovery file holds the open typing burst, a cut waiting for its paste, and any Changes that failed to save. It's updated after every edit and flushed to disk about once a second, so an app crash loses nothing and a computer crash at most about a second of typing. When the Vault opens, every Work with a recovery file is recovered without asking: a draft is recorded as the Change it would have become (at the time of its last keystroke), unsaved Changes are saved, and a pending cut is put back where it was. A line in the Work says what was recovered, and Ctrl+Z reverts it like anything else from the sitting. A recovery file that can't be put back (its History is missing) is kept and reported as a Sync problem.
- The interface and the Rust core exchange positions in UTF-16 code units, as CodeMirror counts them. The core converts them to the engine's characters and rejects any position past the end or inside a character before the engine sees it.
- Syncthing may deliver another Device's Text file before its `.db`. Every Text file ends with a comment line naming the version it was written from, so such a file waits until its Changes arrive instead of being taken for an External edit, and an External edit is compared against exactly the version it started from ([ADR 0010](adr/0010-text-files-name-the-version-they-were-written-from.md)). The line is never part of the Work's text.
- Changes from outside while a Work is open (Syncthing replacing the `.db`, Conflict copies, the Text file edited in another editor) are noticed within about a second. They're taken in through the editor's own queue, in order with the Author's typing, so the editor gets patches and the cursor and any open burst stay where they are. Changes this Device saved that are missing from a `.db` Syncthing delivered are saved into it again. An edit to the Text file waits until no typing burst is open (at most a few seconds).
- An unexpected panic in the engine doesn't close the app: the affected Work's in-memory state is thrown away unsaved and the Work is reopened from its `.db` and the recovery file, with a message to the Author. Other open Works are untouched.

On Debian the app is built on the machine itself and installed as a `.deb` (Tauri can't build it from Windows); `docs/debian.md` lists the packages, the build, and the WebKitGTK rendering workaround.

*Done when:* you can write a short story on Windows and Debian, edit its Text file in vim, sync both machines through Syncthing with offline edits on each, and nothing is lost.

### Phase 3: Typst

- Compiling from the History (only while the Page preview is open), the Page preview, click-to-jump, and the preview following the cursor.
- The Style library as files in `styles/`, with the defaults written into the Vault, library tabs and the "New Story element" form, and Conflict copies of library files reported for the Author to pick.
- The Preamble dialog, from the Work's "Preamble…" button: Looks (the first is the default), extra Styles, the Author's own Story elements added explicitly, settings, and raw Typst as one field with a Merge notice on concurrent edits.
- Styles and Story element definitions recorded in the History just before the first edit in a sitting, the Style update notice, and Milestones rendering with what they had recorded.
- The Text file's generated header, so it compiles on its own.
- Font lookup with bundled fallback fonts and flagged Device fonts, and compile errors in the Page preview.
- PDF Export in a chosen Look, and the `.docx` Export built from Typst's parser.

Steps (1–4 are independent; 6–8 need 5; 9 needs only the Story element definitions from 4):

1. **Compile and Page preview.** One Typst `World` per open Work kept alive between compiles (the 0b lessons: `evict(1)`, lock before compiling, optimised dependencies). Compiles only while the preview is open, about 300 ms after the last keystroke, including the open burst. Pages rendered lazily as SVG. Fonts: Vault, then Device (flagged), then bundled. On errors the last good pages stay, faded, under the error bar. It also compiles as soon as a typing burst is recorded. *Done when:* a 300-page Work previews smoothly while typing, and a stray `#` shows the bar with a working "Show me".
   *Result (2026-11-07):* done. On a 315-page novel: first compile about 0.67 s, recompile after a small edit about 55 ms, plus about 5 ms to check the pages in view (only those are hashed and redrawn; hashing every page cost more than the compile). Scanning the Device's fonts takes about 0.6 s, once per run, in the background when the Vault opens.
2. **Click-to-jump and following the cursor**, with its Vault setting. *Done when:* both directions land on the right spot, including inside a Story element.
   *Result (2026-11-07):* built and tested (in Rust, with a function-based element standing in for Story elements until step 4). The cursor is found glyph by glyph, so it lands on its own line, not at the start of its paragraph; a cursor in markup uses the nearest text. Finding it on page 150 of 315 takes about 8 ms. Still to check by hand in the app.
3. **Completions.** Typing `#` opens a list of what can go there, narrowing as you type, from Typst's own autocompleter (`typst-ide`), so it knows everything in scope at the cursor. The Work's Story elements come first, in their own group with their description; Typst's own functions follow in a group labelled "Typst". Picking one types its template (`#dream[]`, with the cursor inside), as ordinary typing in the open burst. Story elements join the list by themselves once steps 4–5 define them; in step 5, library Story elements the Work hasn't added appear last, marked "not in this Work", and picking one adds it. *Done when:* `#` lists what's available, `#dr` narrows it to `dream`, and picking it types `#dream[]` with the cursor inside, recorded like typing.
   *Result (2026-11-07):* built and tested. Rust is only asked after a `#` (or on Ctrl+Space), and typing more letters narrows the list in the editor without asking again. Until step 4, "Story elements" means anything the Work defines that isn't part of Typst (its own `#let` functions). Picking one replaces the letters typed after the `#`; inside the open burst that only edits the draft, so it's recorded like typing. Still to check by hand in the app.
4. **The Style library as files.** Formats (`.typ`, `.look`, the element header), defaults written into the Vault, change watching, library tabs, the "New Story element" form, Conflict copies with pick-one. *Done when:* a new Vault has the defaults, and editing one in vim is noticed within a second.
   *Result (2026-11-07):* built and tested. Defaults: Styles "Paperback page", "A4 page", "Book text", "Chapter openings", "Page numbers" and "Manuscript"; Looks "Paperback", "Manuscript" and "A4 draft"; Story elements `part`, `scene-break`, `epigraph`, `block-text` and `dedication`, each labelling what it makes. A test compiles every default Look with every default Story element. `vault.db` records each default written (record `defaults`, one field per file holding the version), so a deleted one stays deleted; offering improved versions waits until there is a second version. The library shows in the sidebar, with problems (a Look naming a missing Style, a Story element whose file doesn't define its function) marked and explained. Not done: Typst highlighting and completions in library tabs (they're plain Raw view for now).
5. **The Preamble in the History.** Preamble records, and the Preamble dialog from the Work's "Preamble…" button (also its right-click menu and a shortcut while writing); the compile source built from the Work's own Looks, extra Styles and Story elements; the Look picker in the preview (Device state); recording before the first edit and the Style update notice; the raw Typst Merge notice. A field Change records which value it replaced (`replaces`), so two edits that replaced the same value are known to be concurrent, whatever their times; old rows have none. Story element chips, `***` → `#scene-break()`, and the "Add it?" error chip. *Done when:* switching Looks in the preview works, and a Style edited on another Device shows the notice and is recorded on the next edit.
   *Result (2026-11-08):* built and tested in Rust and the interface; still to try by hand. A field Change records which value it replaced (storage version 9), and two edits of the raw Typst that replaced the same value get a Merge notice holding the version that lost; the Preamble dialog shows it next to the box, and saving settles it. The Preamble is one record of fields, plus one record per Style, Look and Story element used, holding that file's content as last recorded. A new Work starts with the Paperback Look. A changed file is recorded before the first edit after the library changed (not only the first in a sitting), so a Style changed mid-sitting is recorded too. The compile header comes from the library's current versions, falling back to the Work's own copies and then the app's defaults; an error inside it names the library file and its line. Positions (errors, click-to-jump, following the cursor, completions) are shifted past the header, so they stay in the Work's text. The Look picker in the Page preview shows only when a Work has more than one Look; its choice is kept on this Device. Shortcut for the dialog: Ctrl+Shift+L.
6. **The Text file header** ([ADR 0011](adr/0011-text-files-carry-a-generated-header.md)): written and stripped, edits inside it dropped with a line saying so, old Text files without one still read. *Done when:* the `typst` command line compiles the Text file to the same pages as the app.
   *Result (2026-11-09):* built and tested. The engine only knows that a Text file may begin with a generated block between two marker lines, which it takes off when reading (like the version line) and writes when given one; the app builds it from the Work's recorded copies (the library's version only for something never recorded, such as a new Work's), in its first Look. Edits inside it are dropped, with a line saying so (only for a file the app didn't write itself), and edits to the text in the same file are taken in. A Text file from before the header is still read. Tested in Rust by compiling the written Text file on its own, as the command line would, and comparing every page (as SVG) with the app's; the command line itself isn't installed here, so that check is still to do by hand (winget install Typst.Typst, then 	ypst compile --font-path fonts "Rain.typ" in the Vault).
7. **Milestones render as they were:** fields at a Milestone, and a simple "Page preview at Milestone…" picker (the real History view is Phase 5). *Done when:* an old Milestone renders unchanged after a Style edit.
   *Result (2026-11-09):* built and tested. A Milestone is rendered from a Work rebuilt from only the Changes it holds, so its text, Preamble and recorded Styles are exactly as they were (no new engine rule); its header uses only the copies recorded then. The Page preview gets a picker (Now, or a Milestone); a Milestone's pages are read-only (no recompiling while typing, no click-to-jump, no following the cursor) and kept apart from the Work's own. Naming a Milestone records changed Styles first, so it holds what the preview showed. Until the History view (Phase 5), Milestones are named from the Work's right-click menu ("Name a Milestone…"). Tested: pages identical (as SVG) after the library's Style changed and the text changed, with the Work open or closed, and after the library's file was deleted.
8. **PDF Export:** chosen Look, Styles recorded first, refused on errors, date from the History. *Done when:* byte-identical on Windows and Debian.
9. **`.docx` Export** from `typst-syntax`: behaviour menu, manuscript format, the "left out" report. *Done when:* it opens cleanly in Word and LibreOffice.

*Done when:* the same version of a Work, with every font from the Vault or bundled, produces a byte-identical PDF on Windows and Debian, an old Milestone re-renders unchanged after a Style is edited, the Text file compiled with the `typst` command line gives the same pages as the app's Export in the first Look, and the `.docx` opens cleanly in Word and LibreOffice.

### Phase 4: Comments

- Reviewers in `vault.db`, plus adding, replying to and resolving Comments, coloured highlights, and Detached comments.

*Done when:* a Comment survives a Move, a partial deletion, a full deletion (it detaches) and a Revert (it reattaches).

### Phase 5: History view

- Sessions with word-level highlights, Milestones and comparing them, "History of this text", Revert with preview, and returning to a Milestone.

*Done when:* in the "The rain had stopped." → "eased" scenario, reverting the first Change removes the sentence cleanly by default, and leaves "eased" behind only when you ask for that.

### Phase 6: Polish and Android

- The Chapter list with drag-to-reorder, focus mode, word counts, themes, and the spellcheck switch.
- The Android layout (one screen at a time), the "All files access" flow (detect the missing permission and open its settings page; see the 0c lessons).
- Testing: Playwright on Chromium and WebKit, plus hand testing on a real phone with real keyboards.

*Done when:* a full novel goes back and forth between Android, Windows and Debian through Syncthing, with offline edits on all three, and ends with identical text and Comments everywhere.

## Open design questions for Phase 1

Settle these while building the History engine, and write an ADR for any answer that's hard to reverse.

- ~~Two Devices Move the same text to different places while offline.~~ Settled: the later Move in the fixed Change order wins (so it depends on how Change IDs are formed).
- ~~One Device Moves some text while another deletes part of it, or Moves part of it somewhere else.~~ Settled: a deletion wins wherever the text went. With overlapping Moves, only the overlap goes with the winning Move, the rest of each Move still happens, and the Author gets a Merge notice.
- ~~How a Change ID is formed and how a Device gets its ID.~~ Settled: the ID is Device ID + counter. Changes are ordered by a hybrid logical clock (wall-clock time that never goes below anything already seen), then by Device ID. A Device ID is random and short, made on first launch, kept in Device state and left out of Android backups. The History records a readable name for each Device. A merge that finds the same ID with different content refuses loudly.
- ~~How long a gap ends a Session.~~ Settled: each Change stores its Session ID (the ID of the Session's first Change). A new Session starts when the Work is opened on a Device, or after about 30 minutes without a Change (to be tuned in use). A Session never spans Devices or Works.
- How the editor shows Changes merged in from elsewhere without making your cursor jump. Partly settled: an open typing burst is anchored to characters, so merges don't move it. Cursors and selections outside a burst are still open (Phase 2).
- ~~A Style file in `styles/` edited on two Devices.~~ Settled (Phase 3): the Style library stays plain files, so it can be edited in any text editor. A Style is `styles/<name>.typ` (only `set`/`show` rules), a Look is `styles/<name>.look` (Style names in stack order, one per line), and a Story element is `styles/elements/<name>.typ` (a Typst function with a header comment holding its metadata). A Conflict copy of one of these is never merged: it's a Sync problem, and the Author picks a version (keep this one, keep the other, or open both).
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
├── The Long Night.images/      ← regenerated image copies, only if the Work has images (v2)
├── Short stories/              ← ordinary folders, any arrangement
│   ├── Rain.db
│   └── Rain.typ
├── styles/                     ← Style library: Styles (.typ) and Looks (.look)
│   └── elements/               ← Story elements (.typ)
├── themes/                     ← app themes (CSS)
├── fonts/
└── vault.db                    ← marks the Vault: Devices, Reviewers, Vault settings
```
