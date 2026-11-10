# wrtier themes

A theme changes how the app looks: its colours, and the font and width of the text you write in. It never changes a Work's pages; those come from its Looks.

## Using a theme

In the app, open Settings → **Import theme…** and pick the theme's `.css` file. The app copies it into the Vault's `themes/` folder and switches to it. Every Device that opens the Vault can then choose it from the Theme list. Tick "Only on this device" to use a theme on one Device only.

You can also copy a `.css` file into `themes/` yourself. A theme that changes in the folder, from any editor or another Device, shows its new version within about a second.

If the chosen theme's file disappears, the app follows the system's light or dark mode and Settings says the theme is missing.

## Writing a theme

A theme is one CSS file. Its **first line** names it and says whether it's light or dark:

```css
/* Theme: Sepia, light */
```

The app starts from its own light or dark colours and adds your file on top, so set only what you want to change. Everything else stays readable. Write your values in a plain `:root { … }` block; it overrides the app's own whatever selectors those use.

[sepia.css](sepia.css) is a complete example.

## What a theme can rely on

The variables below are kept stable between versions of the app. Any other CSS in a theme also applies, but the app's own class names and layout can change in any update, so rules that use them may break.

**The app**

| Variable | What it colours |
|---|---|
| `--bg` | The main background, behind the text and the Comment cards |
| `--panel` | The sidebar, the right panel, the open tab and tooltips |
| `--text` | Text |
| `--muted` | Quieter text: hints, times, counts |
| `--line` | Dividing lines and borders |
| `--accent` | What's focused or chosen, links, and the cursor's place on the pages |
| `--warn-bg`, `--warn-text` | Notices that need attention |
| `--error-bg`, `--error-text` | Errors |
| `--chip`, `--chip-hover`, `--chip-open` | Story element chips, buttons and the shaded cursor line |

**The writing**

| Variable | Default | What it sets |
|---|---|---|
| `--editor-font-family` | `Georgia, "Iowan Old Style", serif` | The editor's font. Use fonts installed on the Device; the Vault's `fonts/` folder is only for the pages. |
| `--text-width` | `44em` | The widest the text column gets |
| `--editor-line-height` | `1.6` | Line spacing in the editor |

The editor's font size is a setting of its own (Settings → Editor font size), not part of a theme.

**Comments and Reviewers**

Each Reviewer gets one of eight colour slots. Each slot has a colour for names and swatches, and a see-through `-soft` one for highlighting the text of their Comments. Overlapping highlights darken, so keep the soft colours see-through.

| Variable | |
|---|---|
| `--reviewer-teal`, `--reviewer-teal-soft` | |
| `--reviewer-amber`, `--reviewer-amber-soft` | |
| `--reviewer-red`, `--reviewer-red-soft` | |
| `--reviewer-violet`, `--reviewer-violet-soft` | |
| `--reviewer-green`, `--reviewer-green-soft` | |
| `--reviewer-orange`, `--reviewer-orange-soft` | |
| `--reviewer-pink`, `--reviewer-pink-soft` | |
| `--reviewer-brown`, `--reviewer-brown-soft` | |
| `--comment-author`, `--comment-author-soft` | The Author's own Comments |
| `--comment-unknown`, `--comment-unknown-soft` | Comments from a Reviewer this Vault doesn't know |

**History**

| Variable | |
|---|---|
| `--session-tint-0` … `--session-tint-4` | "Colour text by Session": the newest Session's text (0) to the fifth newest (4). Faint and see-through. |

**Typst colouring** (the Raw view and Style library files)

| Variable | What it colours |
|---|---|
| `--syntax-comment` | Comments (`// …`) |
| `--syntax-keyword` | `set`, `show`, `let`, `if` … and operators |
| `--syntax-function` | Function names, such as `#page` or a Story element's `#epigraph` |
| `--syntax-string` | Strings and raw text |
| `--syntax-number` | Numbers and lengths (`148mm`) |
| `--syntax-label` | Labels, references and links |
| `--syntax-punctuation` | Brackets, list markers |
| `--syntax-escape` | Escapes (`\*`) |
| `--syntax-heading` | Headings (also set in bold) |
