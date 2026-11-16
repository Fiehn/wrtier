// Story element: Part
// Written as: #part[Book One]
// Shown as: block
// .docx: heading above chapters
// Groups Chapters: its title stands alone on a page. Styles change it with `show <part>`.

#let part(title) = {
  pagebreak(weak: true)
  v(30%)
  [#block(width: 100%, align(center, text(size: 1.6em, title))) <part>]
  pagebreak(weak: true)
}

// In a .docx: its title is never split across pages or hyphenated.
/* .docx
Part: <w:keepLines/> <w:suppressAutoHyphens/>
*/
