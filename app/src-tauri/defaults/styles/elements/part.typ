// Story element: Part
// Written as: #part[Book One]
// Shown as: block
// .docx: part heading
// Groups Chapters: its title stands alone on a page. Styles change it with `show <part>`.

#let part(title) = {
  pagebreak(weak: true)
  v(30%)
  [#block(width: 100%, align(center, text(size: 1.6em, title))) <part>]
  pagebreak(weak: true)
}
