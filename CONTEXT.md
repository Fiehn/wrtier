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
The **Vault**'s collection of **Styles**, **Looks** and **Story element** definitions, including the defaults the app ships with.
_Avoid_: style store, template folder

**Preamble**:
The part of a **Work**, kept apart from its text, that lists the **Looks** it uses, any extra **Styles** for this **Work** only, its settings (title, author name, language) and any raw Typst for this **Work** only.
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

**Detached comment**:
A **Comment** whose text has all been deleted; it is kept in a side list with a quote of the text it was attached to.
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
_Avoid_: undo (that is the ordinary Ctrl+Z), rollback, delete change

**Session**:
The **Changes** one **Device** made in one sitting, shown together in the history view.
_Avoid_: batch, save

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
A duplicate of a **History** file made by the sync tool when two **Devices** changed it before syncing; it is expected, not an error.
_Avoid_: corruption, sync error

## Relationships

- A **Work** has exactly one **History** and one **Text file**.
- A **Work** is written in one language, set in its **Preamble**; the language decides quotation marks and hyphenation.
- A **Work** contains zero or more **Chapters**; reordering **Chapters** is a **Move**, so their history and **Comments** go with them.
- A **Collection** includes other **Works** live; it never holds copies of them.
- A **Vault** holds many **Works**, many **Reviewers** and one **Style library**.
- A **Reviewer** can comment on any **Work** in the **Vault**.
- The app ships built-in **Story elements**; the **Author** can define more in the **Style library**.
- Every **Story element** has a default appearance; **Styles** change how it looks, and its definition says what a `.docx` **Export** does with it.
- A **Work**'s **History** records the definitions of the **Story elements** it uses, just as it records its **Styles**.
- A **Look** stacks many **Styles**; when two **Styles** set the same thing, the one lower in the stack wins.
- A **Work** uses one or more **Looks** from the **Style library**; its **Preamble** can add extra **Styles** on top for that **Work** only.
- A PDF **Export** uses exactly one **Look**; a `.docx` **Export** ignores **Looks** and keeps only the text's structure and emphasis.
- A **Work**'s **History** records the content of every **Style** it uses each time that **Style** changes, so every **Milestone** renders exactly as it did.
- Fonts are only referenced by name: the app looks in the **Vault**'s fonts folder first, then the **Device**'s fonts, and otherwise warns and uses a default.
- A **Comment** has exactly one sender: the **Author** or one **Reviewer**.
- Replying to a resolved **Comment** doesn't reopen it.
- When two **Devices** set the same setting differently, the later one wins; the other value stays in the **History**. Raw Typst in the **Preamble** is merged like the **Work**'s text, so neither **Device**'s edit is lost.
- **Reviewers** only comment; only the **Author** changes the text.
- An AI is always a **Reviewer**, never an **Author**: it can add **Comments** but can never change the text.
- A **Comment** follows its text through **Moves**. If all its text is deleted it becomes a **Detached comment**; restoring the text reattaches it.
- A **Comment** covers the characters it was attached to, plus any text later added strictly inside them; text added at either edge isn't covered. If part of its text is **Moved** away, the **Comment** covers both parts and stays one **Comment**.
- Text added inside a passage on one **Device** while another **Device** **Moves** that passage ends up inside the passage at its new place.
- When two **Devices** **Move** overlapping text, the text both moved goes with one of the two **Moves**, chosen the same way on every **Device**; the rest of each **Move** still happens, and the **Author** gets a **Merge notice**.
- Text deleted on one **Device** stays deleted even if another **Device** **Moved** it.
- A **Work**'s **History** records the name of every **Reviewer** who commented on it, so the file can be read on its own outside the **Vault**.
- Every character in a **Work** knows which **Change** wrote it.
- A **Session** groups many **Changes**; each **Change** belongs to exactly one **Session**.
- A **Session** belongs to one **Device** and one **Work**. A new one starts when the **Work** is opened, or after a long pause in writing.
- Reverting a **Change** also reverts, by default, every later **Change** that added text inside the text it wrote (and anything added inside that, in turn). Later deletions and **Moves** of that text are not reverted with it.
- A **Revert** can itself be reverted.
- A **Change** stays undone while any **Revert** of it is in effect; restoring a **Change** cancels every **Revert** of it.
- Returning to a **Milestone** reverts every **Change** that isn't part of it; nothing is removed.
- A **Milestone** holds exactly the **Changes** the **Device** that named it had seen. A **Change** made earlier on another **Device** that hadn't synced yet is not part of it.
- A **Work**'s images are stored in its **History**; inserting or replacing an image is a **Change**.
- A **Text file** is always derived from its **History**, never the other way round.
- An **External edit** is recorded into the **History** as a new **Change**; it never replaces the **History**.
- An **External edit** is compared against the text the app last wrote to that **Text file**, so it merges as if it came from a **Device** that had been offline since then.
- A **Conflict copy** is merged back into its **History** by whichever **Device** sees it first, then deleted.
- Every **Change** records which **Device** made it and when. **Changes** are ordered by when they were made, and a **Device** never places a new **Change** before one it has already seen.
- A **Work**'s **History** records a readable name for every **Device** that changed it.
- Settings that belong to one **Device** (app theme, window layout) are never stored in the **Vault**.
- Merging two copies of a **History** keeps every **Change** from both; nothing is ever lost or chosen between.

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
- "editor" can mean the app's text-editing area or a person who reviews the **Work**. Resolved: a person is always a **Reviewer** (with "editor" as a role label); "editor" on its own means the app's text-editing area.
