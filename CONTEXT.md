# Writer (working name)

A local, portable writing app for novels and short stories, with full change history, comments and Typst typesetting, working out of a synced folder on Windows, Debian and Android. Notes, outlines and worldbuilding are deliberately left to other apps.

## Language

### Writing

**Vault**:
The one folder, synced between **Devices**, that holds all of the **Author**'s **Works**, **Reviewers** and **Style library**.
_Avoid_: library, workspace, project

**Work**:
A complete piece of writing treated as a whole (a novel, a short story, or whatever size the **Author** decides) with its own **History**.
_Avoid_: document, project, file, book

**Chapter**:
A section of a **Work** that starts at a top-level heading; part of the Work's text, not a separate file.
_Avoid_: part, section file

**Story element**:
A named kind of text in a **Work** (such as a **Chapter**, a scene break, an epigraph, or one the **Author** defines) that is always written one fixed way.
_Avoid_: block, component, macro, tag

**Collection**:
A **Work** built mainly by including other **Works**, so that changes to the originals show up in it.
_Avoid_: anthology, compilation, bundle

### Editing

**Formatted view**:
The editor mode where Typst markup is hidden and shown as formatting, except where the cursor is.
_Avoid_: live preview, WYSIWYG, rich text

**Raw view**:
The editor mode that shows every bit of Typst markup as typed.
_Avoid_: code view, source mode

**Page preview**:
The compiled pages of a **Work** in one of its **Looks**, shown next to or instead of the editor.
_Avoid_: preview (on its own), PDF view, render

**Export**:
A file made from a **Work** to send out: a PDF in one of its **Looks**, or a plain `.docx` with minimal formatting.
_Avoid_: output, render, build, compile

### Styling

**Style**:
One small, reusable piece of page styling that does a single thing, such as "A5 page" or "Drop caps".
_Avoid_: template, theme, preamble

**Look**:
A named, ordered stack of **Styles** that gives a **Work** its whole appearance, such as "Paperback" or "Submission".
_Avoid_: template, theme, layout, preset

**Style library**:
The **Vault**'s collection of **Styles**, **Looks** and **Story element** definitions, including the defaults the app wrote into it when the **Vault** was made; once written, the defaults belong to the **Vault** and a newer app never changes them.
_Avoid_: style store, template folder

**Style update notice**:
A line shown on a **Work** whose recorded copy of a **Style** or **Story element** definition differs from the **Style library**'s, saying the **Page preview** uses the new version and the next edit records it; showing it records nothing.
_Avoid_: merge notice, style conflict, warning

**Preamble**:
The part of a **Work**, kept apart from its text, that lists the **Looks** it uses, any extra **Styles** for this **Work** only, the **Author**'s own **Story elements** it uses, its settings (title, author name, language) and any raw Typst for this **Work** only.
_Avoid_: header, front matter, settings

### Feedback

**Author**:
The one person who writes a **Work** and the only one whose actions change its text.
_Avoid_: user, owner

**Reviewer**:
A named person, or an AI, whose feedback on a **Work** is recorded as **Comments**; may carry a role label such as "editor", "beta reader" or "AI".
_Avoid_: sender, commenter, critic

**Comment**:
A note attached to a range of text in a **Work**, sent by the **Author** or by a **Reviewer**; it is either open or resolved.
_Avoid_: annotation, note, remark

**Reply**:
A short answer to a **Comment**, with its own sender (the **Author** or a **Reviewer**); it belongs to exactly one **Comment**, is never attached to text, and can't itself be replied to.
_Avoid_: response, sub-comment, thread entry

**Detached comment**:
A **Comment** none of whose text is visible any more (the text it was attached to and anything later added inside it have all been deleted); it is kept in a side list with a quote of the text it was attached to, and reattaches by itself as soon as any of that text is back.
_Avoid_: orphan, deleted comment

### History

**History**:
The append-only record of every **Change** ever made to a **Work**; it is the source of truth.
_Avoid_: versions, log, backups

**Change**:
One uniquely identified, never-edited action in a **History**, such as a burst of typing, a deletion or a **Move**.
_Avoid_: edit, revision, commit, keystroke

**Move**:
A **Change** that relocates existing text so it keeps its history and comments.
_Avoid_: cut and paste

**Revert**:
A **Change** that cancels out an earlier **Change**, which stays in the **History**.
_Avoid_: rollback, delete change, undo (that's the Ctrl+Z key, which may make a Revert)

**Session**:
The **Changes** one **Device** made in one sitting, shown together in the **History view**.
_Avoid_: batch, save

**History view**:
The list of a **Work**'s **Sessions**, **Milestones** and **Merge notices**, newest first, where the **Author** looks back over the **History**, compares **Milestones** and **Reverts** or restores; "History of this text" is the same list narrowed to a stretch of text.
_Avoid_: timeline, log, changes list, version history

**Milestone**:
A named point in a **History** that the **Author** can return to or compare against, e.g. "Draft 2, sent to editor".
_Avoid_: version, snapshot, tag, draft

**Text file**:
The plain `.typ` file regenerated from the **History** so the **Work** can be read, shared and compiled outside the app.
_Avoid_: source file, master copy

**External edit**:
A difference found in a **Text file** that the app did not make itself.

### Sync

**Device**:
Any installation of the app that opens the **Vault**; all **Devices** are equal and none is in charge.
_Avoid_: master, home device, primary

**Merge notice**:
A message shown to the **Author** after a merge, pointing at text the merge had to decide about on its own, such as a passage split by two overlapping **Moves**; it asks for nothing and changes nothing.
_Avoid_: conflict, merge error, warning

**Conflict copy**:
A duplicate of a file in the **Vault** (usually a **History**, but also a **Text file** or a file in the **Style library**) made by the sync tool when two **Devices** changed it before syncing; it is expected, not an error.
_Avoid_: corruption, sync error

**Sync problem**:
A file in the **Vault** the app couldn't take in, such as a **Conflict copy** it couldn't merge or a **History** from a newer version of the app; it is kept untouched and reported until it's resolved.
_Avoid_: conflict, sync error, merge notice

### Settings

**Vault settings**:
Preferences kept in the **Vault** and shared by every **Device**, such as the app theme or the editor font size; any **Device** can override one for itself.
_Avoid_: preferences, shared settings, config

**Device state**:
What one **Device** remembers only for itself and never syncs, such as its **Device** ID, open tabs and cursor positions.
_Avoid_: device settings, local settings

### Mods

**Mod**:
An optional feature that ships with the app, such as spellcheck, a dialogue checker or the **Chapter** list, which the **Author** adds to a **Work** to get more ways to see and work on it.
_Avoid_: plugin, extension, add-on, module

## Relationships

- A **Work** has exactly one **History** and one **Text file**.
- A **Work** is written in one language, set in its **Preamble**; the language decides quotation marks and hyphenation.
- Cutting text and pasting exactly that text elsewhere in the same **Work** is a **Move**, as is dragging it; anything else records the cut as a deletion.
- A **Work** contains zero or more **Chapters**; reordering **Chapters** is a **Move**, so their history and **Comments** go with them.
- A **Collection** includes other **Works** live; it never holds copies of them.
- A **Vault** holds many **Works**, many **Reviewers** and one **Style library**.
- A **Reviewer** can comment on any **Work** in the **Vault**.
- Removing a **Reviewer** from the **Vault** only stops them being offered as a sender; their **Comments** and **Replies** keep their name and colour, and they can be brought back.
- The app ships built-in **Story elements**; the **Author** can define more in the **Style library**.
- Every **Work** has the built-in **Story elements**. The **Author**'s own are added to each **Work** explicitly, in its **Preamble**; a **Work** never picks one up just because its text uses it.
- Every **Story element** has a default appearance; **Styles** change how it looks, and its definition says what a `.docx` **Export** does with it.
- A **Work**'s **History** records the definitions of the **Story elements** it uses, just as it records its **Styles**.
- A **Look** stacks many **Styles**; when two **Styles** set the same thing, the one lower in the stack wins.
- A **Work** uses one or more **Looks** from the **Style library**; its **Preamble** can add extra **Styles** on top for that **Work** only.
- Only one **Look** is rendered at a time: the **Author** picks it in the **Page preview** and again for each PDF **Export**. The **Preamble**'s extra **Styles** and the **Work**'s **Story elements** are rendered in every **Look**.
- A **Look**'s own **Styles** may change how a **Story element** looks, but never define one, so a **Story element** never disappears when the **Look** changes.
- A PDF **Export** uses exactly one **Look**; a `.docx` **Export** ignores **Looks** and keeps only the text's structure and emphasis.
- A **Work**'s **History** records the content of every **Style** it uses each time that **Style** changes, so every **Milestone** renders exactly as it did.
- A changed **Style** is recorded in a **Work** just before a **Device**'s first edit to it in a sitting, and before any **Export**; opening or reading a **Work** never records one. Until then the **Page preview** uses the **Style library**'s version, and the **Work** shows a **Style update notice**.
- **Milestones** always render with the **Styles** the **Work** had recorded at that point, never with the **Style library**'s current ones.
- Fonts are only referenced by name: the app looks in the **Vault**'s fonts folder first, then the **Device**'s fonts, and otherwise warns and uses a default that ships with the app. A font found only on the **Device** is flagged, since other **Devices** may render the **Work** differently.
- The same version of a **Work** with the same fonts gives the same PDF on every **Device**.
- A **Comment** has exactly one sender: the **Author** or one **Reviewer**.
- Every **Reviewer** has one colour from a fixed set, the same in every **Work**; the **Author**'s **Comments** use the app's accent colour.
- Replying to a resolved **Comment** doesn't reopen it.
- A **Comment** has zero or more **Replies**, in the order they were made; deleting a **Comment** removes its **Replies** with it.
- The text and sender of a **Comment** or **Reply** can be changed after it's made. When two **Devices** change the same text without seeing each other's change, the later one wins and the **Author** gets a **Merge notice** that shows the other version, as with the **Preamble**'s raw Typst.
- When two **Devices** set the same setting differently, the later one wins; the other value stays in the **History**. Raw Typst in the **Preamble** works the same way, and when two **Devices** edited it without seeing each other's edit, the **Author** gets a **Merge notice** that shows the other version.
- **Reviewers** only comment; only the **Author** changes the text.
- **Comments** live only in the **Work**'s **History** and the app: they're never written to the **Text file** or to any **Export**.
- An AI is always a **Reviewer**, never an **Author**: it can add **Comments** but can never change the text.
- A **Comment** follows its text through **Moves**. If all its text is deleted it becomes a **Detached comment**; restoring the text reattaches it.
- A **Comment** covers the characters it was attached to, plus any text later added strictly inside them; text added at either edge isn't covered. If part of its text is **Moved** away, the **Comment** covers both parts and stays one **Comment**. Text **Moved** into the middle of it from elsewhere isn't covered, so the highlight has a gap there.
- Text added inside a passage on one **Device** while another **Device** **Moves** that passage ends up inside the passage at its new place.
- When two **Devices** **Move** overlapping text, the text both moved goes with the later of the two **Moves**; the rest of each **Move** still happens, and the **Author** gets a **Merge notice**.
- When two **Devices** **Move** passages into each other, only the earlier **Move** happens; the later one's text stays where it was, and the **Author** gets a **Merge notice**.
- A **Move** takes the text the **Author** saw in the selection; text that an earlier **Move** had already taken out of that passage stays where it went.
- Text deleted on one **Device** stays deleted even if another **Device** **Moved** it.
- A **Work**'s **History** records the name of every **Reviewer** who commented on it, so the file can be read on its own outside the **Vault**.
- Every character in a **Work** knows which **Change** wrote it.
- A **Session** groups many **Changes**; each **Change** belongs to exactly one **Session**.
- A **Session** belongs to one **Device** and one **Work**. A new one starts when the **Work** is opened, or after a long pause in writing.
- A **Session**'s word counts say what it did when it was made; later **Changes** never alter them.
- A **Milestone** named in the middle of a **Session** cuts it in two: the part before is in the **Milestone**, the part after isn't. A **Session** made earlier on a **Device** that hadn't synced isn't in it either, however old it is.
- **Reverting** a **Session** reverts its **Changes** to the text and the **Preamble**; its **Comments**, **Milestones** and the **Styles** it recorded on its own stay.
- Restoring brings back everything one **Revert** took; to keep only part of it, restore it all and **Revert** the rest.
- Comparing two **Milestones**, or one with the **Work** as it is now, compares characters, so **Moved** text shows as a **Move**.
- "History of this text" covers the characters selected and every character that ever lay between them, including text since deleted or **Moved** away; unlike a **Comment**, it keeps text **Moved** out of it.
- Reverting a **Change** also reverts, by default, every later **Change** that added text inside the text it wrote (and anything added inside that, in turn). Later deletions and **Moves** of that text are not reverted with it.
- A **Revert** can itself be reverted.
- Undo (Ctrl+Z) only edits text typed in the last few seconds that isn't a **Change** yet; beyond that it's a **Revert** of the latest **Change** to the text this **Device** made that's still in effect (a **Revert** or a return to a **Milestone** made in the **History view** included), and redo restores it. Undo never touches **Comments**.
- A **Change** stays undone while any **Revert** of it is in effect; restoring a **Change** cancels every **Revert** of it.
- Returning to a **Milestone** reverts every **Change** to the **Work**'s content that isn't part of it, including later **Reverts**; nothing is removed. **Device** names, other **Milestones**, dismissed **Merge notices**, and **Comments** and their **Replies** stay as they are; a **Comment** whose text the return removes becomes a **Detached comment**.
- A **Milestone** holds exactly the **Changes** the **Device** that named it had seen. A **Change** made earlier on another **Device** that hadn't synced yet is not part of it.
- A **Work**'s images are stored in its **History**; inserting or replacing an image is a **Change**.
- A **Text file** is always derived from its **History**, never the other way round.
- A **Text file** compiles on its own: it carries the **Work**'s recorded **Styles**, **Looks** and **Story elements**, rendered in the **Work**'s first **Look**. Changing the **Preamble** happens only in the app.
- An **External edit** is recorded into the **History** as a new **Change**; it never replaces the **History**.
- An **External edit** is compared against the text the app last wrote to that **Text file**, so it merges as if it came from a **Device** that had been offline since then.
- A **Conflict copy** is merged back into its **History** by whichever **Device** sees it first, then deleted, whether or not its **Work** is open. One that can't be merged is kept as a **Sync problem**.
- A **Merge notice** is about a **Work**'s content (its text or its **Preamble**); a **Sync problem** is about a file in the **Vault**.
- A **Conflict copy** of a file in the **Style library** is never merged; it stays a **Sync problem** until the **Author** picks which version to keep.
- Every **Change** records which **Device** made it and when. **Changes** are ordered by when they were made, and a **Device** never places a new **Change** before one it has already seen.
- A **Work**'s **History** records a readable name for every **Device** that changed it, so the file can be read on its own. The app itself always shows a **Device**'s current name from the **Vault**.
- **Device state** is never stored in the **Vault**. **Vault settings** are, including each **Device**'s overrides of them.
- A **Device** knows only its own ID; it finds its name and overrides in the **Vault** by that ID.
- Merging two copies of a **History** keeps every **Change** from both; nothing is ever lost or chosen between.
- **Mods** are added to and removed from each **Work** separately.
- Which **Mods** a **Work** has is kept in the **Vault**, not in its **History**, so returning to a **Milestone** or a **Revert** never adds or removes a **Mod**.
- A new **Work** starts with the **Vault**'s **Mods** for new **Works**.
- A **Device** can hide a **Mod** for itself without removing it from the **Work**.
- A **Device** whose app doesn't have a **Mod** leaves it on the **Work** as it is; a newer app's **Mods** are never lost because an older one changed the list.
- A **Mod** never changes a **Work**'s text or how it renders or exports, so a **Work** gives the same **Text file**, PDF and `.docx` whichever **Mods** it has.
- A **Mod** may keep its own data in the **Vault**; removing it from a **Work** hides that data but never deletes it.
- **Chapters**, **Story elements**, **Styles**, **Looks**, **Comments** and the **History view** are part of the app, not **Mods**.

## Example dialogue

> **Dev:** "If I fix a typo in the **Text file** with vim on Debian, does that overwrite the **History**?"
> **Author:** "No. The app spots it as an **External edit** and adds it to the **History** as a new **Change**."
> **Dev:** "And if my phone and laptop both edited the same **Work** offline?"
> **Author:** "Syncthing makes a **Conflict copy**. Whichever **Device** opens the vault first merges every **Change** from it back in and deletes it."
> **Dev:** "If I fix that typo in a short story, is it fixed in the **Collection** too?"
> **Author:** "Yes. A **Collection** includes the original **Works**; it doesn't copy them."
> **Dev:** "Anna left a **Comment** on a paragraph I then moved to Chapter 6 and deleted. Is her **Comment** gone?"
> **Author:** "No, it's a **Detached comment** now. If I **Revert** the deletion, it reattaches to the same text."
> **Dev:** "Could the AI just fix the paragraph instead of commenting?"
> **Author:** "No. An AI is a **Reviewer**, so all it can do is leave a **Comment**. Only I change the text."

## Flagged ambiguities

- "document" was used for both a whole novel and a single chapter. Resolved: the whole thing is a **Work**; its sections are **Chapters**.
- "preview" was used for both the formatted editor and the compiled pages. Resolved: the editor mode is the **Formatted view**; the compiled pages are the **Page preview**.
- "styling" and "theming" were both used for appearance. Resolved: a **Style** is about the printed page; "theme" means only the app's own appearance.
- "Device settings" was used both for what one Device keeps locally and for preferences such as the theme. Resolved: what never syncs is **Device state**; preferences are **Vault settings**, which a **Device** can override for itself.
- "style element" was used for whatever a **Work** adds outside its **Looks**. Resolved: those are the **Preamble**'s extra **Styles** and the **Work**'s **Story elements**, both rendered in every **Look**.
- "editor" can mean the app's text-editing area or a person who reviews the **Work**. Resolved: a person is always a **Reviewer** (with "editor" as a role label); "editor" on its own means the app's text-editing area.
