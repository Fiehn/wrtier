// Story element: Dedication
// Written as: #dedication[For M.]
// Shown as: block
// The dedication, alone on its page.

#let dedication(body) = {
  pagebreak(weak: true)
  v(30%)
  [#block(width: 100%, align(center, emph(body))) <dedication>]
  pagebreak(weak: true)
}

// In a .docx: never split across pages or hyphenated.
/* .docx
Dedication: <w:keepLines/> <w:suppressAutoHyphens/>
*/
